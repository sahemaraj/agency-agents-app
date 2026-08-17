## Purpose

Provides bounded local discovery and safe plain-text reading of approved catalog playbooks without executing markup, code, Agents, or runbooks.

## ADDED Requirements

### Requirement: Playbooks are discovered only under approved catalog roots
The system SHALL enumerate only normalized UTF-8 Markdown files beneath fixed approved playbook roots. It MUST reject links, reparse points, special entries, traversal, unsupported extensions, excessive depth, excessive count, and oversized files.

#### Scenario: Catalog contains a linked playbook path
- **WHEN** any candidate path component is a symbolic link or reparse point
- **THEN** discovery or reading fails closed without following the link

### Requirement: Playbook reads are source-relative and bounded
The system SHALL expose stable source-relative provenance, title, kind, size, and content within configured bounds and deterministic sort order.

#### Scenario: User searches playbooks
- **WHEN** the user supplies a bounded local search term
- **THEN** matching titles, paths, kinds, or plain-text content are returned deterministically without network access

### Requirement: Catalog markup is inert
The UI SHALL render playbook content as preformatted text and SHALL NOT interpret HTML, Markdown, script, image, or link markup.

#### Scenario: Playbook contains a script element
- **WHEN** content includes `<script>` or other executable-looking markup
- **THEN** the exact characters are displayed as text and no executable DOM element is created

### Requirement: Loading failures retain usable state
The Playbook surface SHALL distinguish loading, empty, error, and ready states, preserve last-known usable data when safe, and offer Retry for failed inspection.

#### Scenario: Refreshing the list fails
- **WHEN** a previously loaded Playbook list cannot be refreshed
- **THEN** the prior list remains visibly stale and the error can be retried

