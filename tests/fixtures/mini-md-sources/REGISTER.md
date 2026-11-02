# Sources fixture

Each test row's ID names a test in this directory; the stale rows name ones that
were renamed or removed.

## Requirements

| ID | Summary |
| --- | --- |
| REQ-1 | Profiles load |
| REQ-2 | The store keeps what it is given |

## Rust tests

| Test | Verifies |
| --- | --- |
| crates/demo/tests/schema.rs::loads_profile | REQ-1 |
| crates/demo/src/lib.rs::tests::splits | REQ-1 |
| crates/demo/tests/schema.rs::loads_old_profile | REQ-1 |
| crates/demo/src/lib.rs::unit::splits | REQ-1 |

## Python tests

| Test | Verifies |
| --- | --- |
| tests/test_store.py::test_put | REQ-2 |
| tests/test_store.py::TestGroup::test_get[case-1] | REQ-2 |
| tests/test_store.py::test_old_put | REQ-2 |
| tests/test_store.py::Outer::Inner::test_deep | REQ-2 |
| tests/test_broken.py::test_any | REQ-2 |

## Go tests

| Test | Verifies |
| --- | --- |
| pkg/store/store_test.go::TestPut | REQ-2 |
| pkg/store/store_test.go::TestOldPut | REQ-2 |

## Documents

| Path | Summary |
| --- | --- |
| docs/guide.md | The guide |
| docs/removed.md | A guide since removed |
