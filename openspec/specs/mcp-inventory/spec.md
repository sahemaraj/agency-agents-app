# mcp-inventory Specification

## Purpose
Provide a bounded privacy-safe inventory and trusted configuration surface for Claude Code and Codex MCP usage without becoming an arbitrary server installer or runtime.
## Requirements
### Requirement: Inventory is bounded, local, and source-aware
The system SHALL inventory only Claude Code and Codex using bounded supported configuration sources. Each server SHALL retain client, bounded name, scope, exact registered project when applicable, transport, privacy-safe endpoint summary, enabled state, environment/header names, and source validation. Entries and issues SHALL be deterministically sorted and capped.

#### Scenario: Supported configurations are combined
- **WHEN** the user refreshes MCP inventory
- **THEN** valid Claude user/project/local entries and Codex JSON-list entries appear once with their source and scope while one failed source does not erase other evidence

#### Scenario: Unsafe config source is isolated
- **WHEN** a config file is linked, non-regular, oversized, malformed, or outside an exact registered project
- **THEN** the system reports one bounded issue, does not follow the unsafe path, and continues with independent sources

### Requirement: Inventory never retains credentials
The system MUST NOT serialize raw environment values, headers, bearer tokens, URLs with userinfo/query/fragment, command arguments, or unbounded config content. It SHALL expose only bounded key names, safe command/host summaries, and an inline-credential warning.

#### Scenario: Inline secret is present
- **WHEN** a configured server includes an inline environment value, authorization header, bearer value, URL secret, or credential-shaped argument
- **THEN** the inventory reports credential presence without including the value in server evidence, errors, logs, or IPC output

### Requirement: Validation is passive and deterministic
The system SHALL classify each entry as valid, warning, or blocked using only configuration evidence. It SHALL validate required fields, supported transports, HTTPS or loopback HTTP endpoints, enabled state, declared filters, and unsafe runtime-launch patterns without starting an MCP server or making a network request.

#### Scenario: External launcher is configured
- **WHEN** a foreign stdio server uses a package/runtime launcher such as `npx`, `uvx`, or Docker
- **THEN** the entry remains inspectable with a warning and no process beyond the supported read-only client inventory command is executed

#### Scenario: Remote endpoint is unsafe
- **WHEN** an HTTP/SSE entry uses an invalid URL, credentials in the URL, or non-loopback cleartext HTTP
- **THEN** the entry is blocked and the serialized endpoint omits sensitive URL parts

### Requirement: Tool discovery is honest
The system SHALL expose the exact current Agency Agents tool names from its existing composed router without launching the server. Foreign entries SHALL expose only declared tool filters and otherwise report tool discovery unavailable.

#### Scenario: Trusted template is inventoried
- **WHEN** Agency Agents is exactly connected to a supported client
- **THEN** its inventory reports the exact sorted unique routed tool names and marks discovery known

#### Scenario: Foreign tools are unknown
- **WHEN** a foreign server declares no tool filters
- **THEN** the UI says tools are unavailable without starting that server and does not imply zero tools

### Requirement: Configuration is restricted to the trusted template
The system SHALL keep automatic connect, repair, and disconnect restricted to the existing Agency Agents registration transaction. Foreign servers SHALL be read-only evidence. Per-project Agency Agents mutation usage SHALL reuse the existing exact canonical project allowlist and per-client policy.

#### Scenario: User manages Agency Agents
- **WHEN** the trusted template is missing, exact, or conflicting
- **THEN** the existing connect, disconnect, or repair action remains available with verified rollback behavior and refreshed inventory

#### Scenario: User inspects a foreign server
- **WHEN** a foreign server appears in inventory
- **THEN** no add, remove, edit, login, execute, or trust action is offered

### Requirement: Existing Settings surface owns the workflow
The existing MCP settings section SHALL show inventory, validation, project usage, trusted-template tools, refresh progress, isolated issues, and accessible announcements while retaining existing policy controls and focus behavior.

#### Scenario: Refresh completes with partial failure
- **WHEN** one client inventory is unavailable
- **THEN** the UI retains other client/server evidence, announces completion with issues, and keeps retry available

#### Scenario: No unrelated authority is added
- **WHEN** Phase 15 is installed
- **THEN** there is no new route, dependency, persisted document, network path, telemetry, notification, arbitrary config mutation, or external server execution path
