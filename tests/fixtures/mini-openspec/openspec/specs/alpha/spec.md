## Purpose
Fixture capability alpha: two requirements, so `contains` fan-out is visible.

## Requirements

### Requirement: Alpha parses input
Alpha SHALL parse its input.

#### Scenario: Well-formed input
- **WHEN** input parses
- **THEN** a value is produced

#### Scenario: Malformed input
- **WHEN** input does not parse
- **THEN** an error is reported

### Requirement: Alpha reports errors
Alpha SHALL report what it could not read.

#### Scenario: One error
- **WHEN** a read fails
- **THEN** the failure is named

### Requirement: Shared title
Alpha's requirement of this title.

#### Scenario: Alpha's
- **WHEN** alpha shares a title
- **THEN** its node is its own
