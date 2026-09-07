# Requirements

## Business Needs

| ID   | Need                        |
|------|-----------------------------|
| BN-1 | Validate traceability       |

## User Needs

| ID   | Need                        | Traces To |
|------|-----------------------------|-----------|
| UN-1 | Run validation from CLI     | BN-1      |

## Domain 01

| ID       | Requirement         | Traces To | Rationale       |
|----------|---------------------|-----------|-----------------|
| REQ-0101 | Parse requirements  | UN-1      | Core feature    |
| REQ-0102 | Parse spec headings | UN-1      | Core feature    |
| REQ-0103 | Trace to criteria   | SC-001    | Acceptance link |

## Success Criteria

| ID     | Criterion                        | Stakeholder | Category  |
|--------|----------------------------------|-------------|-----------|
| SC-001 | Validation runs without errors   | S1          | Must-have |
| SC-002 | Reports are human-readable       | S1, S2      | Should-have |
