## Purpose

Provides complete, previewed, and atomic local security policy presets so a named posture cannot leave hidden client overrides or partially updated mutation permissions.

## ADDED Requirements

### Requirement: Posture classification uses the complete policy matrix
The system SHALL classify Strict and Local Development only when paranoid mode, all six Skill and Agent mutation flags, and every client override exactly match that preset. Any other valid shape SHALL be Custom.

#### Scenario: One client override remains enabled
- **WHEN** global flags match Strict but any client override retains mutation access
- **THEN** the posture is Custom rather than Strict

### Requirement: Preset application is one atomic settings transaction
The system SHALL serialize with concurrent settings writes, load the latest valid settings, update the complete preset matrix, preserve unrelated fields and the project allowlist, clear conflicting client overrides, persist once, and refresh cached state only after success.

#### Scenario: Persistence fails
- **WHEN** writing the preset fails
- **THEN** no partial policy is committed or exposed as active

#### Scenario: Settings are corrupt
- **WHEN** current settings cannot be loaded and validated
- **THEN** preset application fails closed without replacing the corrupt document silently

### Requirement: Presets require complete preview and explicit apply
The UI SHALL show the complete before/after policy matrix and SHALL NOT invoke apply before an explicit user action. Completion or failure SHALL be announced and focus SHALL remain deterministic.

#### Scenario: User opens Strict preview
- **WHEN** the user selects Strict for review
- **THEN** the app shows every affected policy value and performs no write until Apply is activated

