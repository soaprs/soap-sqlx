# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-13

### Added

- PostgreSQL logical-to-physical field mappings with validated identifiers.
- Portable condition, sorting, pagination, and count compilation for SQLx.
- Ordered typed bindings, explicit null ordering, portable empty-set semantics,
  and full unsigned-integer precision through PostgreSQL `numeric` values.
- Adapter-owned entity mappings and codecs that keep domain entities independent
  from SQLx row and column types.
- Generic PostgreSQL implementations of the soaprs `ReadRepository` and
  `WriteRepository` ports.
- Swappable `PgSource` execution boundary and a pool-backed `PgPoolSource`
  implementation for routing, transactions, and instrumentation.
- SQLx and PostgreSQL error classification with stable soaprs error kinds,
  transience metadata, and preserved technical sources.
- Shared CRUD and complete M1 query-contract coverage, plus real PostgreSQL
  integration tests for constraints, mapping failures, and pool timeouts.
- CI quality gates for formatting, Clippy, documentation, Rust 1.94 MSRV, and a
  PostgreSQL service.

[Unreleased]: https://github.com/soaprs/soap-sqlx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/soaprs/soap-sqlx/releases/tag/v0.1.0
