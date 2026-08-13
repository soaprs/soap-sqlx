# soaprs-sqlx

`soaprs-sqlx` provides SQLx database adapters for
[`soaprs`](https://github.com/soaprs/soap). It keeps SQLx, physical identifiers,
and SQL text in infrastructure while application code depends on soaprs ports
and named queries.

The project is an early prototype and is not published yet.

## Current scope

- PostgreSQL logical-field allow-list,
- portable `Condition` and `FindParams` compiler,
- ordered typed bindings using PostgreSQL placeholders,
- explicit null ordering and portable empty-set rewrites,
- SQLx error mapping with preserved technical sources,
- PostgreSQL integration-test infrastructure.

The base `ReadRepository` and `WriteRepository` implementation, entity codecs,
transactions, and native named-query handlers are the next milestones.

## Compatibility

- `soaprs` contracts: `0.1`
- SQLx: `0.9`
- Rust: `1.94` or newer, following the SQLx 0.9 MSRV
- PostgreSQL: 13 or newer

The default feature set enables PostgreSQL and Tokio with rustls/WebPKI TLS:

```toml
[dependencies]
soaprs-sqlx = "0.1"
```

Publication remains disabled until the PostgreSQL repository passes the full
`soaprs-contract-tests` suite against a real database.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
```

Run the ignored PostgreSQL integration test with:

```bash
SOAPRS_POSTGRES_URL=postgres://soaprs:soaprs@localhost:5432/soaprs \
  cargo test --test postgres_integration --all-features -- --ignored
```

## Design boundaries

- Every logical field resolves through an explicit adapter-owned mapping.
- Physical identifiers are validated separately from logical fields.
- Values are always SQLx bindings and never interpolated into SQL text.
- Driver failures map to stable `SoapError` categories and retain their source.
- Application queries never contain raw SQL or SQLx types.
