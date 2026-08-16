//! Agency Agents — Tauri 2 backend entrypoint.
//!
//! Module layout per `memory-bank/backendApi.md` §9. This file is the
//! Tauri Builder + invoke_handler registration; every command lives
//! in `commands::*`.

mod agents;
mod commands;
mod corpus;
mod error;
mod expert_runs;
mod experts;
mod github;
mod install;
mod library;
mod ollama;
mod registry;
mod render;
mod skills;
mod state;
#[allow(dead_code, reason = "Checkpoint A API is consumed by Tasks 4-8")]
mod state_db;
mod types;
mod util;

use commands::*;

pub async fn run_mcp(client: String) -> Result<(), String> {
    skills::mcp::serve(client).await
}

pub async fn run_mcp_http(bind: std::net::SocketAddr, token: String) -> Result<(), String> {
    skills::mcp::serve_http(bind, token).await
}

// =============================================================
// Phase 15 — Updater minisign public key
// =============================================================
//
// The public key half of the minisign keypair used to sign release
// .dmg artifacts. Public keys are public by design — embedding them
// in the binary is the standard pattern for offline-verified updates
// (Sparkle, Tauri, every shipping Mac auto-updater).
//
// **Placeholder.** Replace before cutting a release. The real key is
// generated per `BUILD.md` instructions:
//
//     tauri signer generate -w ~/.config/agency-agents-app/updater.key
//
// The matching public key the command prints is what goes here.
// Keep the private key chmod 600 outside the repo — it's the only
// thing standing between a compromised agencyagents.app and a
// malicious binary push.
//
// Real minisign public key. The matching private key lives at
// `~/.config/agency-agents-app/updater.key` (chmod 600,
// outside the repo). The signature verification at install time
// validates every downloaded `.app.tar.gz` against this pubkey; any
// mismatch aborts the install with no on-disk side effects.
//
// `tauri.conf.json` carries the same value for the plugin to consume
// at startup; keep both in sync. The plugin parses Tauri's base64-of-
// minisign-blob format directly — what you see here is exactly what
// `tauri signer generate -w …` printed.
const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFCRjVBRkQ4ODhFRDI5QkQKUldTOUtlMkkySy8xcTlyRnNjM1pkMy9sc2NkMzdOOVlhTEd5OTVoNFIwWDI4VUROUGhVbFNuMzMK";

pub fn updater_pubkey() -> &'static str {
    UPDATER_PUBKEY
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF renderer aborts with "Could not create default EGL
    // display: EGL_BAD_PARAMETER" on a lot of Linux GPU/driver stacks (Arch,
    // NVIDIA, Wayland, newer Mesa) — the webview never comes up (issue #641).
    // Forcing the non-DMABUF path before GTK/WebKit initializes fixes it, at a
    // negligible rendering cost. Only touch it when the user hasn't set it
    // themselves, so an explicit override still wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Best-effort tracing setup — silent if RUST_LOG is unset.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,agency_agents_app=info")
            }),
        )
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Phase 15 — register the updater plugin. The endpoint URL and
        // public key are configured in `tauri.conf.json`; the plugin
        // pulls them from the parsed Config at startup. Our IPC
        // wrappers in `commands::updater` route every check + install
        // through `state.require_network("update_check")` first so
        // Offline Mode kills the path even though the plugin itself
        // would otherwise try the manifest endpoint.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Issue #17 — persist the window's size + position across launches.
        // The plugin auto-saves geometry when the window is moved/resized and
        // on exit, then restores it on the next launch. Default StateFlags
        // cover size + position (plus maximized/fullscreen) — exactly what the
        // issue asks for. No frontend wiring: registration is the feature.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .menu(build_app_menu)
        .on_menu_event(handle_menu_event)
        // Persist window geometry on every resize/move, not just on graceful
        // exit — so a size change survives even a hard kill (e.g. stopping
        // `tauri dev` with Ctrl-C, which never runs the exit-save handler).
        // The state file is tiny; the OS coalesces the writes during a drag.
        .on_window_event(|window, event| {
            use tauri::Manager;
            use tauri_plugin_window_state::{AppHandleExt, StateFlags};
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_)
            ) {
                let _ = window.app_handle().save_window_state(StateFlags::all());
            }
        })
        .setup(|app| {
            state::initialize(app)?;
            use tauri::Manager;
            if let Err(error) = tauri::async_runtime::block_on(
                state::recover_filesystem_operations(app.handle(), &app.state::<state::AppState>()),
            ) {
                tracing::error!("{error}");
            }
            // Phase 15 — spawn the auto-check scheduler. The task
            // sleeps for 24h between wakes, re-reads the live settings
            // on each cycle (so a user toggling auto-check off mid-run
            // is honoured on the next wake), and runs the check only
            // when both `update_auto_check` is on AND `paranoid_mode`
            // is off. Backoff on failure: 1h → 6h → 24h.
            commands::updater::spawn_auto_check_scheduler(app.handle().clone());
            #[cfg(target_os = "macos")]
            {
                // Apply NSVisualEffectView to the main window so it picks up the
                // native macOS "frosted glass" appearance. Material::HudWindow
                // gives a slightly heavier blur that looks good behind the
                // sidebar and main panes; the WebView background must be set
                // transparent in CSS (see app.css :root) for the blur to show.
                use tauri::Manager;
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                if let Some(window) = app.get_webview_window("main") {
                    let _ = apply_vibrancy(
                        &window,
                        NSVisualEffectMaterial::HudWindow,
                        Some(NSVisualEffectState::Active),
                        None,
                    );
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            settings_get,
            settings_set,
            mcp_policy_set,
            mcp_client_policy_set,
            mcp_agent_policy_set,
            mcp_agent_client_policy_set,
            mcp_clients_status,
            mcp_inventory,
            mcp_client_connect,
            mcp_client_disconnect,
            mcp_client_repair,
            settings_reset,
            doctor_report,
            state::mcp_audit_list,
            state::storage_migration_status,
            state::storage_migration_start,
            state::storage_migration_retry,
            state::storage_visible_revision,
            state::storage_backup,
            state::storage_open_data_directory,
            state::storage_legacy_conflicts_dismiss,
            github_repo_stats,
            github_status,
            github_signin_start,
            github_signin_poll,
            github_signout,
            github_star,
            github_unstar,
            github_is_starred,
            github_watch,
            github_unwatch,
            github_create_issue,
            update_check_now,
            update_install,
            update_skip,
            update_relaunch,
            // Phase 1 — corpus subsystem (contracts.md §C). These live in
            // the `corpus` module rather than `commands::*`; register them
            // fully-qualified alongside the other commands.
            corpus::corpus_status,
            corpus::corpus_refresh,
            corpus::corpus_list,
            corpus::corpus_get,
            corpus::corpus_categories,
            corpus::catalog_source_get,
            corpus::catalog_configured,
            corpus::catalog_source_set,
            corpus::catalog_detect,
            corpus::catalog_provision_managed,
            corpus::catalog_pull,
            corpus::catalog_source_transition_recover,
            corpus::catalog_feed_list,
            corpus::catalog_status,
            corpus::catalog_check_updates,
            corpus::runbooks_list,
            experts::experts_list,
            experts::experts_get,
            experts::expert_save,
            experts::expert_delete,
            experts::expert_import,
            experts::expert_export,
            experts::expert_plan_activation,
            experts::expert_activate,
            experts::expert_activation_history,
            experts::expert_activation_requests,
            experts::expert_activation_request_resolve,
            experts::expert_creation_requests,
            experts::expert_creation_request_get,
            experts::expert_creation_request_approve,
            experts::expert_creation_request_reject,
            expert_runs::expert_runs_list,
            expert_runs::expert_run_get,
            expert_runs::expert_run_review,
            agents::agent_sources_list,
            agents::agent_sources_inspect,
            agents::agent_source_add_local,
            agents::agent_source_add_github,
            agents::agent_source_refresh,
            agents::agent_source_remove,
            agents::agent_source_status,
            agents::agent_get,
            agents::agent_text_read,
            agents::agent_render_preview,
            agents::drafts::agent_drafts_list,
            agents::drafts::agent_draft_get,
            agents::drafts::agent_draft_create,
            agents::drafts::agent_from_skill_preview,
            agents::drafts::agent_draft_edit,
            agents::drafts::agent_draft_publish,
            agents::drafts::agent_draft_reject,
            agents::drafts::agent_draft_duplicate,
            agents::organize::agent_library_list,
            library::task_recommendations,
            agents::organize::agent_folder_create,
            agents::organize::agent_folder_rename,
            agents::organize::agent_folder_move,
            agents::organize::agent_folder_delete,
            agents::organize::agent_folder_assign,
            agents::organize::agent_favorite_set,
            agents::organize::agent_recent_touch,
            agents::organize::agent_collection_save,
            agents::organize::agent_collection_delete,
            agents::organize::agent_smart_folder_save,
            agents::organize::agent_smart_folder_delete,
            agents::organize::agent_profile_save,
            agents::organize::agent_profile_delete,
            agents::organize::agent_library_replace,
            agents::organize::agent_library_export,
            agents::organize::agent_library_import,
            agents::organize::agent_update_policy_set,
            agents::organize::agent_publisher_trust_set,
            agents::organize::agent_preferred_source_set,
            agents::organize::agent_usage_record,
            agents::organize::agent_approval_approve,
            agents::organize::agent_approval_reject,
            ollama::ollama_status,
            ollama::ollama_plan,
            ollama::ollama_apply,
            skills::skill_sources_list,
            skills::skill_sources_inspect,
            skills::skill_trust_grant,
            skills::skill_trust_revoke,
            skills::skill_package_destinations,
            skills::skill_install,
            skills::skill_update,
            skills::skill_disable,
            skills::skill_enable,
            skills::skill_uninstall,
            skills::skill_backups_list,
            skills::skill_version_history_list,
            skills::skill_install_plan,
            skills::skill_install_with_dependencies,
            skills::skill_collection_batch,
            skills::skill_version_rollback,
            skills::skill_installs_reconcile,
            skills::skill_source_add_local,
            skills::skill_source_add_github,
            skills::skill_source_refresh,
            skills::skill_source_remove,
            skills::drafts::skill_drafts_list,
            skills::drafts::skill_draft_publish,
            skills::drafts::skill_draft_reject,
            skills::drafts::skill_draft_create,
            skills::drafts::skill_draft_edit,
            skills::drafts::skill_text_read,
            skills::organize::skill_folders_list,
            skills::organize::skill_folder_create,
            skills::organize::skill_folder_rename,
            skills::organize::skill_folder_move,
            skills::organize::skill_folder_delete,
            skills::organize::skill_folder_assign,
            skills::organize::skill_folders_import,
            skills::organize::skill_favorite_set,
            skills::organize::skill_recent_touch,
            skills::organize::skill_collection_save,
            skills::organize::skill_collection_delete,
            skills::organize::skill_smart_folder_save,
            skills::organize::skill_smart_folder_delete,
            skills::organize::skill_profile_save,
            skills::organize::skill_profile_delete,
            skills::organize::skill_library_replace,
            skills::organize::skill_library_export,
            skills::organize::skill_library_import,
            skills::organize::skill_update_policy_set,
            skills::organize::skill_publisher_trust_set,
            skills::organize::skill_preferred_source_set,
            skills::organize::skill_approval_approve,
            skills::organize::skill_approval_reject,
            // Phase 2 — install + reconcile (contracts.md §C). The cross-tool
            // agent state layer: render/ledger/reconcile/tools/projects.
            install::install_agent,
            install::update_agent,
            install::agent_install_plan,
            install::agent_update_plan,
            install::agent_uninstall_plan,
            install::agent_install_with_dependencies,
            install::agent_batch_install_plan,
            install::agent_batch_apply,
            install::agent_collection_install_plan,
            install::agent_collection_update_plan,
            install::agent_collection_uninstall_plan,
            install::agent_collection_apply,
            install::agent_version_history,
            install::agent_version_rollback,
            install::disable_agent,
            install::enable_agent,
            install::track_agent,
            install::agent_diff,
            install::uninstall_agent,
            install::project_forget,
            install::installs_reconcile,
            install::installs_for_agent,
            install::tools_list,
            install::tool_versions,
            install::reveal_path,
            install::project_register,
            install::project_unregister,
            install::projects_list,
            install::project_baseline_save_team,
            install::project_baseline_import_pack,
            install::project_readiness_get,
            install::project_subscription_set,
            install::project_recommendations_list,
            install::project_recommendation_dismiss,
            install::project_recommendation_open,
            install::project_instructions_inspect,
            install::project_instruction_plan,
            install::project_instruction_apply,
            install::loadout_export,
            install::loadout_import,
            install::loadout_apply,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// =============================================================
// Native macOS menu (Phase 12+)
// =============================================================
//
// macOS apps have a system menu bar above the screen, not inside the window.
// The "App" menu is the first item (named after the app) and is where users
// expect to find "About <App>" and "Settings…". Per Tauri 2 conventions we
// build the menu in a closure passed to `.menu(...)` on the Builder, and
// dispatch click events from `.on_menu_event(...)`.
//
// The menu items emit Tauri events that the frontend listens for via
// `listen()` and turns into store-state updates (`ui.openAbout()` /
// `ui.openSettings()`). This keeps the menu definition Rust-side and the
// modal rendering entirely in Svelte.

const MENU_EVENT_ABOUT: &str = "agency-agents/menu/about";
const MENU_EVENT_SETTINGS: &str = "agency-agents/menu/settings";

fn build_app_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};

    let pkg = app.package_info();

    // App menu: About (custom — opens our in-app modal), Settings…, ─, Hide
    // / Hide-Others / Show-All, ─, Quit. The native PredefinedMenuItem::about
    // would open the OS dialog; we route through our own modal instead via
    // a MenuItemBuilder + the menu event so the donate CTA + Anthropic
    // credits render in our UI.
    let about_item = MenuItemBuilder::new(format!("About {}", pkg.name))
        .id(MENU_EVENT_ABOUT)
        .build(app)?;
    let settings_item = MenuItemBuilder::new("Settings…")
        .id(MENU_EVENT_SETTINGS)
        .accelerator("CmdOrCtrl+,")
        .build(app)?;

    let app_submenu = SubmenuBuilder::new(app, pkg.name.clone())
        .item(&about_item)
        .separator()
        .item(&settings_item)
        .separator()
        .item(&PredefinedMenuItem::hide(app, None)?)
        .item(&PredefinedMenuItem::hide_others(app, None)?)
        .item(&PredefinedMenuItem::show_all(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;

    // Standard ancillary menus — Edit (copy/paste/etc.) + Window. Pure
    // PredefinedMenuItems so we don't have to reinvent them.
    let edit_submenu = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let window_submenu = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?)
        .build()?;

    MenuBuilder::new(app)
        .item(&app_submenu)
        .item(&edit_submenu)
        .item(&window_submenu)
        .build()
}

fn handle_menu_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: tauri::menu::MenuEvent) {
    use tauri::Emitter;
    match event.id().as_ref() {
        MENU_EVENT_ABOUT => {
            let _ = app.emit("menu:about", ());
        }
        MENU_EVENT_SETTINGS => {
            let _ = app.emit("menu:settings", ());
        }
        _ => {}
    }
}
