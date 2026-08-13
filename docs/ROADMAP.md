# Roadmap

## M0 — PostgreSQL compiler foundation

- PostgreSQL identifier and logical-field mapping
- portable condition, sorting, and pagination compilation
- typed bindings and SQL injection boundary
- SQLx error classification
- real PostgreSQL smoke test in CI

## M1 — repository adapter

- explicit entity row codec contract
- `ReadRepository`: `get`, `find`, and `count`
- `WriteRepository`: `insert`, `replace`, and `remove`
- complete `soaprs-contract-tests` conformance on PostgreSQL
- unique, foreign-key, timeout, and mapping error integration tests

## M2 — native queries and transactions

- infrastructure-owned native named-query handlers
- transaction prototype without changing `soaprs-core` prematurely
- documented retry and ambiguous-write behavior

## Publication criterion

The crate remains private until the complete repository contract passes against
a real PostgreSQL service and every supported/unsupported semantic difference is
documented.
