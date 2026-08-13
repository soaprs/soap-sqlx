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
- adapter-owned entity mappings and row codecs,
- generic PostgreSQL `ReadRepository` and `WriteRepository`,
- swappable `PgSource` execution boundary with a `PgPoolSource` implementation,
- shared CRUD and complete M1 query-contract coverage,
- PostgreSQL integration tests for database and pool failures.

Transactions and infrastructure-owned native named-query handlers are the next
milestones.

`PgSource` is intentionally PostgreSQL-specific. Routing between PostgreSQL,
MongoDB, an HTTP service, or an entity cache belongs above adapters as a type
implementing the neutral `ReadRepository`/`WriteRepository` ports. This keeps
backend capabilities out of a lowest-common-denominator source interface.

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

Publication remains disabled until CI confirms the full
`soaprs-contract-tests` suite against a real database.

## Repository construction

The application continues to depend on `ReadRepository<E>` or
`WriteRepository<E>`. Infrastructure composes the PostgreSQL adapter from a
repository, source, and entity codec:

```rust,ignore
let source: Arc<dyn PgSource> = Arc::new(PgPoolSource::new(pool));
let codec: Arc<dyn PgEntityCodec<User>> = Arc::new(UserCodec::new()?);
let users = PgRepository::new(source, codec)?;
```

For the common case, `PgRepository::from_pool(pool, codec)` creates the same
composition. A custom `PgSource` can select a pool, hold a transaction, add
telemetry, or route PostgreSQL reads and writes without changing the repository
or its consumers.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
```

Run the ignored PostgreSQL integration tests with:

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
- Domain entities do not implement SQLx traits; infrastructure codecs own row
  decoding and scalar encoding.
- `PgRepository` does not own a pool directly; connection selection, routing,
  transactions, and instrumentation belong behind `PgSource`.
