# Roadmap

## M0 — PostgreSQL compiler foundation

Status: implemented; PostgreSQL execution is verified by CI.

- PostgreSQL identifier and logical-field mapping
- portable condition, sorting, and pagination compilation
- typed bindings and SQL injection boundary
- SQLx error classification
- real PostgreSQL smoke test in CI

## M1 — repository adapter

Status: released in `0.1.0`.

- explicit entity row codec contract
- pool-independent `PgSource` execution boundary
- `ReadRepository`: `get`, `find`, and `count`
- `WriteRepository`: `insert`, `replace`, and `remove`
- complete `soaprs-contract-tests` conformance on PostgreSQL
- unique, foreign-key, timeout, and mapping error integration tests

## M2 — native queries and transactions

Status: implemented for `0.2.0`.

- infrastructure-owned native named-query handlers
- shared transaction source without changing `soaprs-core`
- primary/replica source with explicit read/write intent
- documented retry and ambiguous-write behavior

## M3 — PostgreSQL production hardening

Status: next.

- transaction isolation levels and savepoints
- configurable constraint-to-domain-error mapping
- additional PostgreSQL types such as arrays, ranges, network addresses, and
  application enums when demanded by real adapters
- batch persistence and streaming APIs where neutral soaprs ports allow them
- optional codec derive support in a separate proc-macro crate

## Publication criterion

Every release must pass the complete repository contract and adapter-specific
integration tests against a real PostgreSQL service. Supported and unsupported
semantic differences must remain explicit.
