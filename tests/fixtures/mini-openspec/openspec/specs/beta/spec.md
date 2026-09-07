## Purpose
Fixture capability beta: one requirement with three scenarios, so the scenario
count attr is distinguishable from a default.

## Requirements

### Requirement: Beta holds state
Beta SHALL hold its state.

#### Scenario: First
- **WHEN** state is set
- **THEN** it is held

#### Scenario: Second
- **WHEN** state is read
- **THEN** it is returned

#### Scenario: Third
- **WHEN** state is cleared
- **THEN** it is empty

### Requirement: Beta survives restart
No test cites this requirement, so it is the uncovered one COVERAGE reports.

#### Scenario: Restart
- **WHEN** beta restarts
- **THEN** state is intact

