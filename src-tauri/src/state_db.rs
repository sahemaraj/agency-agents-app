use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, ErrorCode, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::PersistenceDocument;
use crate::types::StorageMigrationState;

pub(crate) const SCHEMA_VERSION: u32 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_FILESYSTEM_OPERATION_BYTES: usize = 1024 * 1024;
type ImportParser = dyn Fn(&[u8]) -> Result<String, AppError> + Send + Sync + 'static;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FilesystemOperationPhase {
    Prepared,
    FilesystemApplied,
    Committed,
}

impl FilesystemOperationPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::FilesystemApplied => "filesystem_applied",
            Self::Committed => "committed",
        }
    }

    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "filesystem_applied" => Ok(Self::FilesystemApplied),
            "committed" => Ok(Self::Committed),
            _ => Err(AppError::StorageCorrupt {
                message: "filesystem operation phase is invalid".into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilesystemOperation {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) phase: FilesystemOperationPhase,
    pub(crate) payload: serde_json::Value,
    pub(crate) recovery_error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct StateDatabase {
    path: PathBuf,
}

#[derive(Clone, Copy)]
pub(crate) struct DocumentSpec<T> {
    name: &'static str,
    version: u32,
    max_bytes: u64,
    validate: fn(&T) -> Result<(), AppError>,
    marker: PhantomData<fn() -> T>,
}

#[derive(Clone)]
pub(crate) struct ImportSpec {
    name: &'static str,
    empty_payload: String,
    parse_and_validate: std::sync::Arc<ImportParser>,
}

impl ImportSpec {
    pub(crate) fn new(
        name: &'static str,
        empty_payload: &'static str,
        parse_and_validate: fn(&[u8]) -> Result<String, AppError>,
    ) -> Self {
        Self {
            name,
            empty_payload: empty_payload.into(),
            parse_and_validate: std::sync::Arc::new(parse_and_validate),
        }
    }

    pub(crate) fn document<T>(spec: DocumentSpec<T>, empty: T) -> Self
    where
        T: DeserializeOwned + Serialize + Send + Sync + 'static,
    {
        let empty_payload = serde_json::to_string(&empty).expect("serialize empty state document");
        Self {
            name: spec.name,
            empty_payload,
            parse_and_validate: std::sync::Arc::new(move |raw| {
                let value =
                    serde_json::from_slice::<T>(raw).map_err(|_| AppError::StorageCorrupt {
                        message: format!("{} legacy state is malformed", spec.name),
                    })?;
                (spec.validate)(&value).map_err(|_| AppError::StorageCorrupt {
                    message: format!("{} legacy state is invalid", spec.name),
                })?;
                serde_json::to_string(&value).map_err(|_| AppError::Internal {
                    message: format!("serialize {} migration state", spec.name),
                })
            }),
        }
    }
}

pub(crate) struct ImportOutcome {
    pub(crate) backup_dir: PathBuf,
}

pub(crate) struct StorageLease {
    _file: File,
}

impl StorageLease {
    pub(crate) fn shared(app_data_dir: &Path) -> Result<Self, AppError> {
        let file = open_lock_file(app_data_dir)?;
        file.lock_shared().map_err(|error| AppError::Io {
            message: format!("lock application storage: {error}"),
        })?;
        Ok(Self { _file: file })
    }

    pub(crate) fn try_exclusive(app_data_dir: &Path) -> Result<Self, AppError> {
        let file = open_lock_file(app_data_dir)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(AppError::StorageBusy),
            Err(std::fs::TryLockError::Error(error)) => Err(AppError::Io {
                message: format!("lock application storage: {error}"),
            }),
        }
    }
}

impl<T> DocumentSpec<T> {
    pub(crate) const fn new(
        name: &'static str,
        version: u32,
        max_bytes: u64,
        validate: fn(&T) -> Result<(), AppError>,
    ) -> Self {
        Self {
            name,
            version,
            max_bytes,
            validate,
            marker: PhantomData,
        }
    }
}

impl StateDatabase {
    pub(crate) fn existing(app_data_dir: &Path) -> Option<Self> {
        let path = app_data_dir.join("state").join("agency-agents.sqlite3");
        path.exists().then_some(Self { path })
    }

    pub(crate) async fn completed(app_data_dir: &Path) -> Result<Option<Self>, AppError> {
        let app_data_dir = app_data_dir.to_path_buf();
        run_blocking(move || Self::completed_blocking(&app_data_dir)).await
    }

    pub(crate) fn completed_blocking(app_data_dir: &Path) -> Result<Option<Self>, AppError> {
        let Some(database) = Self::existing(app_data_dir) else {
            return Ok(None);
        };
        match database.migration_state_blocking()? {
            StorageMigrationState::Complete => Ok(Some(database)),
            StorageMigrationState::Legacy
            | StorageMigrationState::InProgress
            | StorageMigrationState::Corrupt => Ok(None),
            StorageMigrationState::Unsupported => Err(AppError::StorageUnsupported {
                found: SCHEMA_VERSION.saturating_add(1),
                supported: SCHEMA_VERSION,
            }),
        }
    }

    pub(crate) fn open(app_data_dir: &Path) -> Result<Self, AppError> {
        let state_dir = app_data_dir.join("state");
        std::fs::create_dir_all(&state_dir).map_err(|error| AppError::Io {
            message: format!("create state directory: {error}"),
        })?;
        let schema_lock = open_named_lock(app_data_dir, "schema.lock")?;
        schema_lock.lock().map_err(|error| AppError::Io {
            message: format!("lock storage schema: {error}"),
        })?;
        let database = Self {
            path: state_dir.join("agency-agents.sqlite3"),
        };
        let connection = database.connection()?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(map_sqlite_error)?;
        if version > SCHEMA_VERSION {
            return Err(AppError::StorageUnsupported {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        if version == 0 {
            connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE IF NOT EXISTS app_meta (
                       key TEXT PRIMARY KEY,
                       value TEXT NOT NULL
                     ) STRICT;
                     CREATE TABLE IF NOT EXISTS state_documents (
                       name TEXT PRIMARY KEY,
                       document_version INTEGER NOT NULL,
                       revision INTEGER NOT NULL,
                       payload TEXT NOT NULL CHECK (json_valid(payload)),
                       updated_at TEXT NOT NULL
                     ) STRICT;
                     CREATE TABLE IF NOT EXISTS filesystem_operations (
                       id TEXT PRIMARY KEY,
                       kind TEXT NOT NULL,
                       phase TEXT NOT NULL CHECK (phase IN ('prepared','filesystem_applied','committed')),
                       payload TEXT NOT NULL CHECK (json_valid(payload)),
                       recovery_error TEXT,
                       created_at TEXT NOT NULL,
                       updated_at TEXT NOT NULL
                     ) STRICT;
                     CREATE TABLE IF NOT EXISTS legacy_imports (
                       name TEXT PRIMARY KEY,
                       relative_path TEXT NOT NULL UNIQUE,
                       was_present INTEGER NOT NULL CHECK (was_present IN (0, 1)),
                       size_bytes INTEGER NOT NULL,
                       sha256 TEXT NOT NULL,
                       imported_at TEXT NOT NULL
                     ) STRICT;
                     INSERT OR IGNORE INTO app_meta (key, value) VALUES ('state_revision', '0');
                     INSERT OR IGNORE INTO app_meta (key, value) VALUES ('storage_migration_state', 'legacy');
                     PRAGMA user_version = 1;
                     COMMIT;",
                )
                .map_err(map_sqlite_error)?;
        }
        drop(connection);
        set_private_permissions(&database.path)?;
        Ok(database)
    }

    pub(crate) async fn read<T>(&self, spec: DocumentSpec<T>) -> Result<Option<T>, AppError>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            let row = connection
                .query_row(
                    "SELECT document_version, payload FROM state_documents WHERE name = ?1",
                    [spec.name],
                    |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(map_sqlite_error)?;
            row.map(|(version, payload)| decode(&spec, version, payload))
                .transpose()
        })
        .await
    }

    pub(crate) fn read_blocking<T>(&self, spec: DocumentSpec<T>) -> Result<Option<T>, AppError>
    where
        T: DeserializeOwned,
    {
        let connection = open_connection(&self.path)?;
        let row = connection
            .query_row(
                "SELECT document_version, payload FROM state_documents WHERE name = ?1",
                [spec.name],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(map_sqlite_error)?;
        row.map(|(version, payload)| decode(&spec, version, payload))
            .transpose()
    }

    pub(crate) async fn mutate<T, R>(
        &self,
        spec: DocumentSpec<T>,
        default: T,
        mutation: impl FnOnce(&mut T) -> Result<R, AppError> + Send + 'static,
    ) -> Result<R, AppError>
    where
        T: DeserializeOwned + Serialize + Send + 'static,
        R: Send + 'static,
    {
        self.mutate_with_revision(spec, default, mutation, true, None)
            .await
    }

    pub(crate) async fn mutate_quiet<T, R>(
        &self,
        spec: DocumentSpec<T>,
        default: T,
        mutation: impl FnOnce(&mut T) -> Result<R, AppError> + Send + 'static,
    ) -> Result<R, AppError>
    where
        T: DeserializeOwned + Serialize + Send + 'static,
        R: Send + 'static,
    {
        self.mutate_with_revision(spec, default, mutation, false, None)
            .await
    }

    pub(crate) async fn mutate_after_filesystem<T>(
        &self,
        spec: DocumentSpec<T>,
        default: T,
        operation_id: &str,
        mutation: impl FnOnce(&mut T) -> Result<(), AppError> + Send + 'static,
    ) -> Result<(), AppError>
    where
        T: DeserializeOwned + Serialize + Send + 'static,
    {
        self.mutate_with_revision(spec, default, mutation, true, Some(operation_id.to_owned()))
            .await
    }

    async fn mutate_with_revision<T, R>(
        &self,
        spec: DocumentSpec<T>,
        default: T,
        mutation: impl FnOnce(&mut T) -> Result<R, AppError> + Send + 'static,
        increment_visible_revision: bool,
        filesystem_operation_id: Option<String>,
    ) -> Result<R, AppError>
    where
        T: DeserializeOwned + Serialize + Send + 'static,
        R: Send + 'static,
    {
        let path = self.path.clone();
        run_blocking(move || {
            let mut connection = open_connection(&path)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_sqlite_error)?;
            if let Some(operation_id) = &filesystem_operation_id {
                let phase = transaction
                    .query_row(
                        "SELECT phase FROM filesystem_operations WHERE id = ?1",
                        [operation_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(map_sqlite_error)?
                    .ok_or_else(|| AppError::InvalidArgument {
                        message: "filesystem operation does not exist".into(),
                    })?;
                if FilesystemOperationPhase::parse(&phase)?
                    != FilesystemOperationPhase::Prepared
                {
                    return Err(AppError::StorageCorrupt {
                        message: "filesystem operation is not prepared".into(),
                    });
                }
            }
            let row = transaction
                .query_row(
                    "SELECT document_version, revision, payload FROM state_documents WHERE name = ?1",
                    [spec.name],
                    |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(map_sqlite_error)?;
            let (mut document, revision) = match row {
                Some((version, revision, payload)) => {
                    (decode(&spec, version, payload)?, revision)
                }
                None => (default, 0),
            };
            let result = mutation(&mut document)?;
            (spec.validate)(&document)?;
            let payload = serde_json::to_string(&document).map_err(|_| AppError::Internal {
                message: "serialize application state".into(),
            })?;
            if payload.len() as u64 > spec.max_bytes {
                return Err(AppError::InvalidArgument {
                    message: format!("{} exceeds its {}-byte limit", spec.name, spec.max_bytes),
                });
            }
            let revision = revision.checked_add(1).ok_or_else(|| AppError::Internal {
                message: "document revision exhausted".into(),
            })?;
            let visible_revision = if increment_visible_revision {
                Some(current_revision(&transaction)?.checked_add(1).ok_or_else(|| {
                    AppError::Internal {
                        message: "state revision exhausted".into(),
                    }
                })?)
            } else {
                None
            };
            transaction
                .execute(
                    "INSERT INTO state_documents \
                     (name, document_version, revision, payload, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(name) DO UPDATE SET \
                       document_version = excluded.document_version, \
                       revision = excluded.revision, payload = excluded.payload, \
                       updated_at = excluded.updated_at",
                    params![
                        spec.name,
                        spec.version,
                        revision,
                        payload,
                        chrono::Utc::now().to_rfc3339()
                    ],
                )
                .map_err(map_sqlite_error)?;
            if let Some(visible_revision) = visible_revision {
                transaction
                    .execute(
                        "UPDATE app_meta SET value = ?1 WHERE key = 'state_revision'",
                        [visible_revision.to_string()],
                    )
                    .map_err(map_sqlite_error)?;
            }
            if let Some(operation_id) = &filesystem_operation_id {
                transaction
                    .execute(
                        "UPDATE filesystem_operations SET phase = 'filesystem_applied', \
                         recovery_error = NULL, updated_at = ?1 WHERE id = ?2",
                        params![chrono::Utc::now().to_rfc3339(), operation_id],
                    )
                    .map_err(map_sqlite_error)?;
            }
            transaction.commit().map_err(map_sqlite_error)?;
            Ok(result)
        })
        .await
    }

    pub(crate) async fn visible_revision(&self) -> Result<u64, AppError> {
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            current_revision(&connection)
        })
        .await
    }

    pub(crate) async fn prepare_filesystem_operation<T: Serialize + ?Sized>(
        &self,
        kind: &str,
        payload: &T,
    ) -> Result<FilesystemOperation, AppError> {
        validate_filesystem_operation_kind(kind)?;
        let payload = serde_json::to_string(payload).map_err(|_| AppError::InvalidArgument {
            message: "filesystem operation payload is invalid".into(),
        })?;
        if payload.len() > MAX_FILESYSTEM_OPERATION_BYTES {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "filesystem operation exceeds its {MAX_FILESYSTEM_OPERATION_BYTES}-byte limit"
                ),
            });
        }
        let id = Uuid::new_v4().to_string();
        let kind = kind.to_owned();
        let path = self.path.clone();
        let record_id = id.clone();
        let record_kind = kind.clone();
        let record_payload = payload.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            let timestamp = chrono::Utc::now().to_rfc3339();
            connection
                .execute(
                    "INSERT INTO filesystem_operations \
                     (id, kind, phase, payload, recovery_error, created_at, updated_at) \
                     VALUES (?1, ?2, 'prepared', ?3, NULL, ?4, ?4)",
                    params![record_id, record_kind, record_payload, timestamp],
                )
                .map_err(map_sqlite_error)?;
            Ok(())
        })
        .await?;
        Ok(FilesystemOperation {
            id,
            kind,
            phase: FilesystemOperationPhase::Prepared,
            payload: serde_json::from_str(&payload).map_err(|_| AppError::Internal {
                message: "deserialize prepared filesystem operation".into(),
            })?,
            recovery_error: None,
        })
    }

    pub(crate) async fn pending_filesystem_operations(
        &self,
    ) -> Result<Vec<FilesystemOperation>, AppError> {
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            let mut statement = connection
                .prepare(
                    "SELECT id, kind, phase, payload, recovery_error \
                     FROM filesystem_operations WHERE phase != 'committed' \
                     ORDER BY created_at, id",
                )
                .map_err(map_sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(map_sqlite_error)?;
            let mut operations = Vec::new();
            for row in rows {
                let (id, kind, phase, payload, recovery_error) = row.map_err(map_sqlite_error)?;
                if payload.len() > MAX_FILESYSTEM_OPERATION_BYTES {
                    return Err(AppError::StorageCorrupt {
                        message: "filesystem operation payload exceeds its limit".into(),
                    });
                }
                validate_filesystem_operation_kind(&kind).map_err(|_| {
                    AppError::StorageCorrupt {
                        message: "filesystem operation kind is invalid".into(),
                    }
                })?;
                operations.push(FilesystemOperation {
                    id,
                    kind,
                    phase: FilesystemOperationPhase::parse(&phase)?,
                    payload: serde_json::from_str(&payload).map_err(|_| {
                        AppError::StorageCorrupt {
                            message: "filesystem operation payload is invalid".into(),
                        }
                    })?,
                    recovery_error,
                });
            }
            Ok(operations)
        })
        .await
    }

    pub(crate) async fn mark_filesystem_applied(&self, id: &str) -> Result<(), AppError> {
        self.advance_filesystem_operation(
            id,
            FilesystemOperationPhase::Prepared,
            FilesystemOperationPhase::FilesystemApplied,
        )
        .await
    }

    pub(crate) async fn commit_filesystem_operation(&self, id: &str) -> Result<(), AppError> {
        self.advance_filesystem_operation(
            id,
            FilesystemOperationPhase::FilesystemApplied,
            FilesystemOperationPhase::Committed,
        )
        .await
    }

    pub(crate) async fn abort_filesystem_operation(&self, id: &str) -> Result<(), AppError> {
        self.advance_filesystem_operation(
            id,
            FilesystemOperationPhase::Prepared,
            FilesystemOperationPhase::Committed,
        )
        .await
    }

    pub(crate) async fn retain_filesystem_operation_error(
        &self,
        id: &str,
        error: &str,
    ) -> Result<(), AppError> {
        let id = id.to_owned();
        let error = error.chars().take(1024).collect::<String>();
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            let changed = connection
                .execute(
                    "UPDATE filesystem_operations SET recovery_error = ?1, updated_at = ?2 \
                     WHERE id = ?3 AND phase != 'committed'",
                    params![error, chrono::Utc::now().to_rfc3339(), id],
                )
                .map_err(map_sqlite_error)?;
            if changed == 0 {
                return Err(AppError::InvalidArgument {
                    message: "pending filesystem operation does not exist".into(),
                });
            }
            Ok(())
        })
        .await
    }

    async fn advance_filesystem_operation(
        &self,
        id: &str,
        expected: FilesystemOperationPhase,
        next: FilesystemOperationPhase,
    ) -> Result<(), AppError> {
        let id = id.to_owned();
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            let current = connection
                .query_row(
                    "SELECT phase FROM filesystem_operations WHERE id = ?1",
                    [&id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(map_sqlite_error)?
                .ok_or_else(|| AppError::InvalidArgument {
                    message: "filesystem operation does not exist".into(),
                })?;
            let current = FilesystemOperationPhase::parse(&current)?;
            if current == next || current == FilesystemOperationPhase::Committed {
                return Ok(());
            }
            if current != expected {
                return Err(AppError::StorageCorrupt {
                    message: "filesystem operation phase transition is invalid".into(),
                });
            }
            connection
                .execute(
                    "UPDATE filesystem_operations \
                     SET phase = ?1, recovery_error = NULL, updated_at = ?2 WHERE id = ?3",
                    params![next.as_str(), chrono::Utc::now().to_rfc3339(), id],
                )
                .map_err(map_sqlite_error)?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn migration_state(&self) -> Result<StorageMigrationState, AppError> {
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            migration_state(&connection)
        })
        .await
    }

    pub(crate) fn migration_state_blocking(&self) -> Result<StorageMigrationState, AppError> {
        migration_state(&self.connection()?)
    }

    pub(crate) async fn import_legacy(
        &self,
        app_data_dir: &Path,
        inventory: &[PersistenceDocument],
        specifications: &[ImportSpec],
    ) -> Result<ImportOutcome, AppError> {
        self.import_legacy_with_hook(app_data_dir, inventory, specifications, || Ok(()))
            .await
    }

    async fn import_legacy_with_hook(
        &self,
        app_data_dir: &Path,
        inventory: &[PersistenceDocument],
        specifications: &[ImportSpec],
        before_complete: impl FnOnce() -> Result<(), AppError> + Send + 'static,
    ) -> Result<ImportOutcome, AppError> {
        ensure_complete_registry(inventory, specifications)?;
        let database_path = self.path.clone();
        let app_data_dir = app_data_dir.to_path_buf();
        let inventory = inventory.to_vec();
        let specifications = specifications.to_vec();
        let result = run_blocking(move || {
            import_legacy_blocking(
                &database_path,
                &app_data_dir,
                &inventory,
                &specifications,
                before_complete,
            )
        })
        .await;
        if matches!(result, Err(AppError::StorageCorrupt { .. })) {
            self.set_migration_state(StorageMigrationState::Corrupt)
                .await?;
        }
        result
    }

    pub(crate) async fn backup_to(&self, destination: &Path) -> Result<(), AppError> {
        let source = self.path.clone();
        let destination = destination.to_path_buf();
        run_blocking(move || backup_database(&source, &destination)).await
    }

    pub(crate) async fn legacy_conflicts(
        &self,
        app_data_dir: &Path,
        inventory: &[PersistenceDocument],
    ) -> Result<Vec<String>, AppError> {
        let database_path = self.path.clone();
        let app_data_dir = app_data_dir.to_path_buf();
        let inventory = inventory.to_vec();
        run_blocking(move || legacy_conflicts(&database_path, &app_data_dir, &inventory)).await
    }

    pub(crate) async fn dismiss_legacy_conflicts(
        &self,
        app_data_dir: &Path,
        inventory: &[PersistenceDocument],
    ) -> Result<(), AppError> {
        let database_path = self.path.clone();
        let app_data_dir = app_data_dir.to_path_buf();
        let inventory = inventory.to_vec();
        run_blocking(move || {
            let connection = open_connection(&database_path)?;
            let conflicts = legacy_conflicts_raw(&connection, &app_data_dir, &inventory)?;
            let fingerprint = conflict_fingerprint(&conflicts);
            connection
                .execute(
                    "INSERT INTO app_meta (key, value) VALUES ('dismissed_legacy_conflicts', ?1) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [fingerprint],
                )
                .map_err(map_sqlite_error)?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn set_migration_state(
        &self,
        state: StorageMigrationState,
    ) -> Result<(), AppError> {
        let path = self.path.clone();
        run_blocking(move || {
            let connection = open_connection(&path)?;
            connection
                .execute(
                    "UPDATE app_meta SET value = ?1 WHERE key = 'storage_migration_state'",
                    [migration_state_name(state)],
                )
                .map_err(map_sqlite_error)?;
            Ok(())
        })
        .await
    }

    fn connection(&self) -> Result<Connection, AppError> {
        open_connection(&self.path)
    }
}

fn validate_filesystem_operation_kind(kind: &str) -> Result<(), AppError> {
    if matches!(
        kind,
        "skill_publish"
            | "skill_install"
            | "skill_update"
            | "skill_disable"
            | "skill_enable"
            | "skill_uninstall"
            | "agent_publish"
            | "agent_install"
            | "agent_update"
            | "agent_disable"
            | "agent_enable"
            | "agent_uninstall"
            | "expert_activate"
            | "workspace_pack_apply"
            | "project_instruction_apply"
    ) {
        Ok(())
    } else {
        Err(AppError::InvalidArgument {
            message: "filesystem operation kind is invalid".into(),
        })
    }
}

fn open_connection(path: &Path) -> Result<Connection, AppError> {
    let connection = Connection::open(path).map_err(map_sqlite_error)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(map_sqlite_error)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(map_sqlite_error)?;
    Ok(connection)
}

fn current_revision(connection: &Connection) -> Result<u64, AppError> {
    let raw: String = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'state_revision'",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)?;
    raw.parse().map_err(|_| AppError::StorageCorrupt {
        message: "state revision is invalid".into(),
    })
}

fn migration_state(connection: &Connection) -> Result<StorageMigrationState, AppError> {
    let raw: String = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'storage_migration_state'",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)?;
    match raw.as_str() {
        "legacy" => Ok(StorageMigrationState::Legacy),
        "in_progress" => Ok(StorageMigrationState::InProgress),
        "complete" => Ok(StorageMigrationState::Complete),
        "corrupt" => Ok(StorageMigrationState::Corrupt),
        "unsupported" => Ok(StorageMigrationState::Unsupported),
        _ => Err(AppError::StorageCorrupt {
            message: "migration state is invalid".into(),
        }),
    }
}

fn migration_state_name(state: StorageMigrationState) -> &'static str {
    match state {
        StorageMigrationState::Legacy => "legacy",
        StorageMigrationState::InProgress => "in_progress",
        StorageMigrationState::Complete => "complete",
        StorageMigrationState::Corrupt => "corrupt",
        StorageMigrationState::Unsupported => "unsupported",
    }
}

fn ensure_complete_registry(
    inventory: &[PersistenceDocument],
    specifications: &[ImportSpec],
) -> Result<(), AppError> {
    let names = specifications
        .iter()
        .map(|specification| specification.name)
        .collect::<HashSet<_>>();
    if names.len() != specifications.len()
        || inventory.len() != specifications.len()
        || inventory
            .iter()
            .any(|document| !names.contains(document.name))
    {
        return Err(AppError::InvalidArgument {
            message: "SQLite migration validators are incomplete".into(),
        });
    }
    Ok(())
}

struct PreparedImport {
    document: PersistenceDocument,
    payload: String,
    was_present: bool,
    size_bytes: u64,
    sha256: String,
}

fn import_legacy_blocking(
    database_path: &Path,
    app_data_dir: &Path,
    inventory: &[PersistenceDocument],
    specifications: &[ImportSpec],
    before_complete: impl FnOnce() -> Result<(), AppError>,
) -> Result<ImportOutcome, AppError> {
    let _lease = StorageLease::try_exclusive(app_data_dir)?;
    let backup_dir = app_data_dir
        .join("state/legacy-backups")
        .join(chrono::Utc::now().format("%Y%m%dT%H%M%S%.fZ").to_string());
    std::fs::create_dir_all(&backup_dir).map_err(|error| AppError::Io {
        message: format!("create legacy backup: {error}"),
    })?;
    set_private_directory_permissions(
        backup_dir
            .parent()
            .expect("legacy backup directory has parent"),
    )?;
    set_private_directory_permissions(&backup_dir)?;
    let mut prepared = Vec::with_capacity(inventory.len());
    for document in inventory {
        let specification = specifications
            .iter()
            .find(|specification| specification.name == document.name)
            .expect("registry completeness checked");
        let relative = safe_relative_path(document.relative_path)?;
        let source = app_data_dir.join(relative);
        if std::fs::symlink_metadata(&source)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(corrupt(document.name));
        }
        let (raw, present) = match std::fs::read(&source) {
            Ok(raw) => (raw, true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (specification.empty_payload.as_bytes().to_vec(), false)
            }
            Err(error) => {
                return Err(AppError::Io {
                    message: format!("read legacy application state: {error}"),
                })
            }
        };
        if raw.len() as u64 > document.max_bytes {
            return Err(corrupt(document.name));
        }
        let payload = (specification.parse_and_validate)(&raw)?;
        if payload.len() as u64 > document.max_bytes {
            return Err(corrupt(document.name));
        }
        let sha256 = hex::encode(Sha256::digest(if present { raw.as_slice() } else { &[] }));
        if present {
            let target = backup_dir.join(relative);
            let parent = target.parent().expect("backup file has parent");
            std::fs::create_dir_all(parent).map_err(|error| AppError::Io {
                message: format!("create legacy backup: {error}"),
            })?;
            set_private_directory_permissions(parent)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
                .map_err(|error| AppError::Io {
                    message: format!("create legacy backup: {error}"),
                })?;
            set_private_permissions(&target)?;
            file.write_all(&raw).map_err(|error| AppError::Io {
                message: format!("write legacy backup: {error}"),
            })?;
            file.sync_all().map_err(|error| AppError::Io {
                message: format!("sync legacy backup: {error}"),
            })?;
            let copied = std::fs::read(&target).map_err(|error| AppError::Io {
                message: format!("verify legacy backup: {error}"),
            })?;
            if copied.len() != raw.len() || hex::encode(Sha256::digest(&copied)) != sha256 {
                return Err(AppError::StorageCorrupt {
                    message: "legacy backup verification failed".into(),
                });
            }
        }
        prepared.push(PreparedImport {
            document: *document,
            payload,
            was_present: present,
            size_bytes: if present { raw.len() as u64 } else { 0 },
            sha256,
        });
    }

    let mut connection = open_connection(database_path)?;
    if migration_state(&connection)? == StorageMigrationState::Complete {
        return Err(AppError::InvalidArgument {
            message: "SQLite migration is already complete".into(),
        });
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            "UPDATE app_meta SET value = 'in_progress' WHERE key = 'storage_migration_state'",
            [],
        )
        .map_err(map_sqlite_error)?;
    let imported_at = chrono::Utc::now().to_rfc3339();
    for item in prepared {
        transaction
            .execute(
                "INSERT OR REPLACE INTO state_documents \
                 (name, document_version, revision, payload, updated_at) \
                 VALUES (?1, ?2, 1, ?3, ?4)",
                params![
                    item.document.name,
                    item.document.version,
                    item.payload,
                    imported_at
                ],
            )
            .map_err(map_sqlite_error)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO legacy_imports \
                 (name, relative_path, was_present, size_bytes, sha256, imported_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    item.document.name,
                    item.document.relative_path,
                    item.was_present,
                    i64::try_from(item.size_bytes).map_err(|_| corrupt(item.document.name))?,
                    item.sha256,
                    imported_at
                ],
            )
            .map_err(map_sqlite_error)?;
    }
    before_complete()?;
    transaction
        .execute(
            "UPDATE app_meta SET value = 'complete' WHERE key = 'storage_migration_state'",
            [],
        )
        .map_err(map_sqlite_error)?;
    transaction.commit().map_err(map_sqlite_error)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if integrity != "ok" {
        return Err(AppError::StorageCorrupt {
            message: "storage integrity check failed".into(),
        });
    }
    Ok(ImportOutcome { backup_dir })
}

fn legacy_conflicts(
    database_path: &Path,
    app_data_dir: &Path,
    inventory: &[PersistenceDocument],
) -> Result<Vec<String>, AppError> {
    let connection = open_connection(database_path)?;
    let conflicts = legacy_conflicts_raw(&connection, app_data_dir, inventory)?;
    let dismissed = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'dismissed_legacy_conflicts'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    if dismissed.as_deref() == Some(&conflict_fingerprint(&conflicts)) {
        Ok(Vec::new())
    } else {
        Ok(conflicts)
    }
}

fn legacy_conflicts_raw(
    connection: &Connection,
    app_data_dir: &Path,
    inventory: &[PersistenceDocument],
) -> Result<Vec<String>, AppError> {
    if migration_state(connection)? != StorageMigrationState::Complete {
        return Err(AppError::InvalidArgument {
            message: "legacy conflicts are available only after SQLite migration".into(),
        });
    }
    let mut conflicts = Vec::new();
    for document in inventory {
        let (was_present, expected_size, expected_hash) = connection
            .query_row(
                "SELECT was_present, size_bytes, sha256 FROM legacy_imports WHERE name = ?1",
                [document.name],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sqlite_error)?
            .ok_or_else(|| AppError::StorageCorrupt {
                message: "legacy fingerprint inventory is incomplete".into(),
            })?;
        let source = app_data_dir.join(safe_relative_path(document.relative_path)?);
        if !was_present {
            if std::fs::symlink_metadata(&source).is_ok() {
                conflicts.push(document.relative_path.to_string());
            }
            continue;
        }
        let bytes = match std::fs::symlink_metadata(&source) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                conflicts.push(document.relative_path.to_string());
                continue;
            }
            Ok(_) => match std::fs::read(&source) {
                Ok(bytes) => bytes,
                Err(_) => {
                    conflicts.push(document.relative_path.to_string());
                    continue;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => {
                conflicts.push(document.relative_path.to_string());
                continue;
            }
        };
        let actual_size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        let actual_hash = hex::encode(Sha256::digest(&bytes));
        if actual_size != expected_size || actual_hash != expected_hash {
            conflicts.push(document.relative_path.to_string());
        }
    }
    Ok(conflicts)
}

fn conflict_fingerprint(conflicts: &[String]) -> String {
    let mut digest = Sha256::new();
    for conflict in conflicts {
        digest.update((conflict.len() as u64).to_le_bytes());
        digest.update(conflict.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn safe_relative_path(path: &str) -> Result<&Path, AppError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::InvalidArgument {
            message: "invalid legacy state path".into(),
        });
    }
    Ok(path)
}

fn backup_database(source: &Path, destination: &Path) -> Result<(), AppError> {
    if destination.exists() {
        return Err(AppError::InvalidArgument {
            message: "backup destination already exists".into(),
        });
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| AppError::Io {
            message: format!("create storage backup directory: {error}"),
        })?;
        set_private_directory_permissions(parent)?;
    }
    let source = open_connection(source)?;
    let expected_revision = current_revision(&source)?;
    let mut destination_connection = Connection::open(destination).map_err(map_sqlite_error)?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut destination_connection)
            .map_err(map_sqlite_error)?;
        backup
            .run_to_completion(64, Duration::from_millis(10), None)
            .map_err(map_sqlite_error)?;
    }
    drop(destination_connection);
    set_private_permissions(destination)?;
    let backup = Connection::open(destination).map_err(map_sqlite_error)?;
    let integrity: String = backup
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    if integrity != "ok" || current_revision(&backup)? != expected_revision {
        return Err(AppError::StorageCorrupt {
            message: "storage backup verification failed".into(),
        });
    }
    Ok(())
}

fn open_lock_file(app_data_dir: &Path) -> Result<File, AppError> {
    open_named_lock(app_data_dir, "storage.lock")
}

fn open_named_lock(app_data_dir: &Path, name: &str) -> Result<File, AppError> {
    let state_dir = app_data_dir.join("state");
    std::fs::create_dir_all(&state_dir).map_err(|error| AppError::Io {
        message: format!("create state directory: {error}"),
    })?;
    let path = state_dir.join(name);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| AppError::Io {
            message: format!("open application storage lock: {error}"),
        })?;
    set_private_permissions(&path)?;
    Ok(file)
}

fn decode<T>(spec: &DocumentSpec<T>, version: u32, payload: String) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    if version > spec.version {
        return Err(AppError::StorageUnsupported {
            found: version,
            supported: spec.version,
        });
    }
    if version != spec.version || payload.len() as u64 > spec.max_bytes {
        return Err(corrupt(spec.name));
    }
    let document = serde_json::from_str(&payload).map_err(|_| corrupt(spec.name))?;
    (spec.validate)(&document).map_err(|_| corrupt(spec.name))?;
    Ok(document)
}

fn corrupt(name: &str) -> AppError {
    AppError::StorageCorrupt {
        message: format!("{name} is invalid"),
    }
}

fn map_sqlite_error(error: rusqlite::Error) -> AppError {
    match error {
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            AppError::StorageBusy
        }
        _ => AppError::Internal {
            message: "storage database operation failed".into(),
        },
    }
}

async fn run_blocking<T>(
    operation: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| AppError::Internal {
            message: "storage task failed".into(),
        })?
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        AppError::Io {
            message: format!("protect storage database: {error}"),
        }
    })
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        AppError::Io {
            message: format!("protect storage directory: {error}"),
        }
    })
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::state::PersistenceDocument;
    use crate::types::StorageMigrationState;
    use rusqlite::{params, Connection};
    use serde::{Deserialize, Serialize};
    use std::time::{Duration, Instant};

    #[derive(Debug, Default, Deserialize, PartialEq, Serialize)]
    struct TestDocument {
        values: Vec<String>,
    }

    fn validate(document: &TestDocument) -> Result<(), AppError> {
        if document.values.iter().any(|value| value.is_empty()) {
            return Err(AppError::InvalidArgument {
                message: "empty test value".into(),
            });
        }
        Ok(())
    }

    fn spec(max_bytes: u64) -> DocumentSpec<TestDocument> {
        DocumentSpec::new("test", 1, max_bytes, validate)
    }

    fn import_json(bytes: &[u8]) -> Result<String, AppError> {
        let document: TestDocument =
            serde_json::from_slice(bytes).map_err(|_| AppError::StorageCorrupt {
                message: "synthetic document is invalid".into(),
            })?;
        validate(&document).map_err(|_| AppError::StorageCorrupt {
            message: "synthetic document is invalid".into(),
        })?;
        serde_json::to_string(&document).map_err(Into::into)
    }

    fn import_spec(name: &'static str) -> ImportSpec {
        ImportSpec::new(name, r#"{"values":[]}"#, import_json)
    }

    const SYNTHETIC_INVENTORY: &[PersistenceDocument] = &[
        PersistenceDocument {
            name: "one",
            relative_path: "state/one.json",
            version: 1,
            max_bytes: 1024,
            parser: "json",
            validator: "synthetic",
        },
        PersistenceDocument {
            name: "two",
            relative_path: "state/two.json",
            version: 1,
            max_bytes: 1024,
            parser: "json",
            validator: "synthetic",
        },
    ];

    fn insert_payload(database: &StateDatabase, payload: &str) {
        let connection = Connection::open(&database.path).expect("open test database");
        connection
            .execute(
                "INSERT OR REPLACE INTO state_documents \
                 (name, document_version, revision, payload, updated_at) \
                 VALUES ('test', 1, 1, ?1, 'now')",
                params![payload],
            )
            .expect("insert test payload");
    }

    #[tokio::test]
    async fn creates_file_backed_schema_and_distinguishes_absent_from_empty() {
        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        assert!(database.path.is_file());

        let connection = Connection::open(&database.path).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        for table in [
            "app_meta",
            "state_documents",
            "filesystem_operations",
            "legacy_imports",
        ] {
            let count: u32 = connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing {table}");
        }
        drop(connection);

        assert_eq!(database.read(spec(1024)).await.unwrap(), None);
        database
            .mutate(spec(1024), TestDocument::default(), |_| Ok(()))
            .await
            .unwrap();
        assert_eq!(
            database.read(spec(1024)).await.unwrap(),
            Some(TestDocument::default())
        );
        assert_eq!(database.visible_revision().await.unwrap(), 1);
    }

    #[test]
    fn rejects_a_newer_schema() {
        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        let connection = Connection::open(&database.path).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        drop(connection);

        assert!(matches!(
            StateDatabase::open(root.path()),
            Err(AppError::StorageUnsupported { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_corrupt_semantically_invalid_and_oversized_documents() {
        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();

        insert_payload(&database, "{}");
        assert!(matches!(
            database.read(spec(1024)).await,
            Err(AppError::StorageCorrupt { .. })
        ));

        insert_payload(&database, r#"{"values":[""]}"#);
        assert!(matches!(
            database.read(spec(1024)).await,
            Err(AppError::StorageCorrupt { .. })
        ));

        insert_payload(&database, r#"{"values":["123456789"]}"#);
        assert!(matches!(
            database.read(spec(8)).await,
            Err(AppError::StorageCorrupt { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        let mode = std::fs::metadata(database.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn a_busy_writer_times_out_without_changing_data() {
        let root = tempfile::tempdir().unwrap();
        let first = StateDatabase::open(root.path()).unwrap();
        let second = StateDatabase::open(root.path()).unwrap();
        let lock = Connection::open(&first.path).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();

        let started = Instant::now();
        let result = second
            .mutate(spec(1024), TestDocument::default(), |document| {
                document.values.push("blocked".into());
                Ok(())
            })
            .await;
        assert!(matches!(result, Err(AppError::StorageBusy)));
        assert!(started.elapsed() >= Duration::from_secs(4));
        assert!(started.elapsed() < Duration::from_secs(6));
        drop(lock);
        assert_eq!(first.read(spec(1024)).await.unwrap(), None);
    }

    #[tokio::test]
    async fn concurrent_independent_instances_preserve_both_mutations() {
        let root = tempfile::tempdir().unwrap();
        let first = StateDatabase::open(root.path()).unwrap();
        let second = StateDatabase::open(root.path()).unwrap();

        let left = first.mutate(spec(1024), TestDocument::default(), |document| {
            document.values.push("left".into());
            Ok(())
        });
        let right = second.mutate(spec(1024), TestDocument::default(), |document| {
            document.values.push("right".into());
            Ok(())
        });
        let (left, right) = tokio::join!(left, right);
        left.unwrap();
        right.unwrap();

        let mut values = first.read(spec(1024)).await.unwrap().unwrap().values;
        values.sort();
        assert_eq!(values, ["left", "right"]);
        assert_eq!(first.visible_revision().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn rejected_mutation_keeps_the_last_committed_document() {
        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        database
            .mutate(spec(1024), TestDocument::default(), |document| {
                document.values.push("durable".into());
                Ok(())
            })
            .await
            .unwrap();

        let result = database
            .mutate(spec(1024), TestDocument::default(), |document| {
                document.values.push(String::new());
                Ok(())
            })
            .await;

        assert!(matches!(result, Err(AppError::InvalidArgument { .. })));
        assert_eq!(
            database.read(spec(1024)).await.unwrap().unwrap().values,
            ["durable"]
        );
        assert_eq!(database.visible_revision().await.unwrap(), 1);
    }

    #[test]
    fn shared_process_lease_blocks_exclusive_cutover() {
        let root = tempfile::tempdir().unwrap();
        let shared = StorageLease::shared(root.path()).unwrap();
        assert!(matches!(
            StorageLease::try_exclusive(root.path()),
            Err(AppError::StorageBusy)
        ));
        drop(shared);
        StorageLease::try_exclusive(root.path()).unwrap();
    }

    #[test]
    fn concurrent_first_startup_initializes_one_valid_schema() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_path_buf();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let first_barrier = barrier.clone();
        let first_path = path.clone();
        let first = std::thread::spawn(move || {
            first_barrier.wait();
            StateDatabase::open(&first_path)
        });
        let second = std::thread::spawn(move || {
            barrier.wait();
            StateDatabase::open(&path)
        });

        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    }

    #[tokio::test]
    async fn import_requires_a_complete_registry_and_backs_up_exact_legacy_files() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("one.json"), r#"{"values":["kept"]}"#).unwrap();
        let database = StateDatabase::open(root.path()).unwrap();

        assert!(matches!(
            database
                .import_legacy(root.path(), SYNTHETIC_INVENTORY, &[import_spec("one")])
                .await,
            Err(AppError::InvalidArgument { .. })
        ));
        assert_eq!(
            database.migration_state().await.unwrap(),
            StorageMigrationState::Legacy
        );

        let outcome = database
            .import_legacy(
                root.path(),
                SYNTHETIC_INVENTORY,
                &[import_spec("one"), import_spec("two")],
            )
            .await
            .unwrap();
        assert_eq!(
            database.migration_state().await.unwrap(),
            StorageMigrationState::Complete
        );
        assert_eq!(
            std::fs::read_to_string(outcome.backup_dir.join("state/one.json")).unwrap(),
            r#"{"values":["kept"]}"#
        );
        assert!(!outcome.backup_dir.join("state/two.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&outcome.backup_dir)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(outcome.backup_dir.join("state/one.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let one = DocumentSpec::new("one", 1, 1024, validate);
        let two = DocumentSpec::new("two", 1, 1024, validate);
        assert_eq!(database.read(one).await.unwrap().unwrap().values, ["kept"]);
        assert_eq!(
            database.read(two).await.unwrap().unwrap(),
            TestDocument::default()
        );
    }

    #[tokio::test]
    async fn malformed_or_oversized_legacy_data_never_completes_migration() {
        for (contents, max_bytes) in [("not json", 1024), (r#"{"values":[]}"#, 4)] {
            let root = tempfile::tempdir().unwrap();
            let state_dir = root.path().join("state");
            std::fs::create_dir_all(&state_dir).unwrap();
            std::fs::write(state_dir.join("one.json"), contents).unwrap();
            let database = StateDatabase::open(root.path()).unwrap();
            let inventory = [PersistenceDocument {
                max_bytes,
                ..SYNTHETIC_INVENTORY[0]
            }];

            assert!(matches!(
                database
                    .import_legacy(root.path(), &inventory, &[import_spec("one")])
                    .await,
                Err(AppError::StorageCorrupt { .. })
            ));
            assert_eq!(
                database.migration_state().await.unwrap(),
                StorageMigrationState::Corrupt
            );
            assert_eq!(database.read(spec(1024)).await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn interruption_before_complete_rolls_back_import() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("one.json"), r#"{"values":["legacy"]}"#).unwrap();
        let database = StateDatabase::open(root.path()).unwrap();

        let result = database
            .import_legacy_with_hook(
                root.path(),
                &SYNTHETIC_INVENTORY[..1],
                &[import_spec("one")],
                || {
                    Err(AppError::Internal {
                        message: "injected interruption".into(),
                    })
                },
            )
            .await;
        assert!(result.is_err());
        assert_eq!(
            database.migration_state().await.unwrap(),
            StorageMigrationState::Legacy
        );
        let document = DocumentSpec::new("one", 1, 1024, validate);
        assert_eq!(database.read(document).await.unwrap(), None);
        assert_eq!(
            std::fs::read_to_string(state_dir.join("one.json")).unwrap(),
            r#"{"values":["legacy"]}"#
        );

        database
            .import_legacy(
                root.path(),
                &SYNTHETIC_INVENTORY[..1],
                &[import_spec("one")],
            )
            .await
            .unwrap();
        assert_eq!(
            database.migration_state().await.unwrap(),
            StorageMigrationState::Complete
        );
    }

    #[tokio::test]
    async fn backup_failure_leaves_legacy_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("one.json"), r#"{"values":["legacy"]}"#).unwrap();
        std::fs::write(state_dir.join("legacy-backups"), "not a directory").unwrap();
        let database = StateDatabase::open(root.path()).unwrap();

        assert!(matches!(
            database
                .import_legacy(
                    root.path(),
                    &SYNTHETIC_INVENTORY[..1],
                    &[import_spec("one")],
                )
                .await,
            Err(AppError::Io { .. })
        ));
        assert_eq!(
            database.migration_state().await.unwrap(),
            StorageMigrationState::Legacy
        );
        let document = DocumentSpec::new("one", 1, 1024, validate);
        assert_eq!(database.read(document).await.unwrap(), None);
    }

    #[tokio::test]
    async fn modified_legacy_files_are_reported_after_cutover() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("one.json"), r#"{"values":["original"]}"#).unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        database
            .import_legacy(
                root.path(),
                SYNTHETIC_INVENTORY,
                &[import_spec("one"), import_spec("two")],
            )
            .await
            .unwrap();

        assert!(database
            .legacy_conflicts(root.path(), SYNTHETIC_INVENTORY)
            .await
            .unwrap()
            .is_empty());
        std::fs::write(state_dir.join("two.json"), "").unwrap();
        assert_eq!(
            database
                .legacy_conflicts(root.path(), SYNTHETIC_INVENTORY)
                .await
                .unwrap(),
            ["state/two.json"]
        );
        database
            .dismiss_legacy_conflicts(root.path(), SYNTHETIC_INVENTORY)
            .await
            .unwrap();
        assert!(database
            .legacy_conflicts(root.path(), SYNTHETIC_INVENTORY)
            .await
            .unwrap()
            .is_empty());
        std::fs::remove_file(state_dir.join("two.json")).unwrap();
        std::fs::write(state_dir.join("one.json"), r#"{"values":["changed"]}"#).unwrap();
        assert_eq!(
            database
                .legacy_conflicts(root.path(), SYNTHETIC_INVENTORY)
                .await
                .unwrap(),
            ["state/one.json"]
        );
    }

    #[tokio::test]
    async fn online_backup_contains_committed_wal_state_and_passes_integrity_check() {
        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        database
            .mutate(spec(1024), TestDocument::default(), |document| {
                document.values.push("committed".into());
                Ok(())
            })
            .await
            .unwrap();
        let backup = root.path().join("backups/state.sqlite3");

        database.backup_to(&backup).await.unwrap();

        let connection = Connection::open(&backup).unwrap();
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        let revision: String = connection
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'state_revision'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(integrity, "ok");
        assert_eq!(revision, "1");
    }

    #[tokio::test]
    async fn filesystem_operation_phases_are_durable_bounded_and_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let database = StateDatabase::open(root.path()).unwrap();
        let operation = database
            .prepare_filesystem_operation(
                "skill_publish",
                &serde_json::json!({
                    "root": "published",
                    "destination": "reviewer",
                    "expectedHash": "a".repeat(64)
                }),
            )
            .await
            .unwrap();

        let pending = database.pending_filesystem_operations().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, operation.id);
        assert_eq!(pending[0].phase, FilesystemOperationPhase::Prepared);

        let operation_id = operation.id.clone();
        database
            .mutate_after_filesystem(
                spec(1024),
                TestDocument::default(),
                &operation_id,
                |document| {
                    document.values.push("published".into());
                    Ok(())
                },
            )
            .await
            .unwrap();
        assert_eq!(
            database.read(spec(1024)).await.unwrap().unwrap().values,
            ["published"]
        );
        database
            .commit_filesystem_operation(&operation.id)
            .await
            .unwrap();
        database
            .commit_filesystem_operation(&operation.id)
            .await
            .unwrap();
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());

        let oversized =
            serde_json::json!({"payload": "x".repeat(MAX_FILESYSTEM_OPERATION_BYTES + 1)});
        assert!(matches!(
            database
                .prepare_filesystem_operation("skill_publish", &oversized)
                .await,
            Err(AppError::InvalidArgument { .. })
        ));

        let aborted = database
            .prepare_filesystem_operation("agent_publish", &serde_json::json!({}))
            .await
            .unwrap();
        database
            .abort_filesystem_operation(&aborted.id)
            .await
            .unwrap();
        database
            .abort_filesystem_operation(&aborted.id)
            .await
            .unwrap();
        assert!(database
            .pending_filesystem_operations()
            .await
            .unwrap()
            .is_empty());

        let failed = database
            .prepare_filesystem_operation("expert_activate", &serde_json::json!({}))
            .await
            .unwrap();
        database
            .retain_filesystem_operation_error(&failed.id, "changed hash")
            .await
            .unwrap();
        assert_eq!(
            database.pending_filesystem_operations().await.unwrap()[0]
                .recovery_error
                .as_deref(),
            Some("changed hash")
        );
    }
}
