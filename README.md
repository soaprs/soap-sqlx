# soaprs-sqlx

`soaprs-sqlx` provides SQLx database adapters for
[`soaprs`](https://github.com/soaprs/soap). It keeps SQLx, physical identifiers,
and SQL text in infrastructure while application code depends on soaprs ports
and named queries.

The crate is published on crates.io. Version `0.6` aligns the adapter with the
`soaprs 0.6` contracts while preserving the PostgreSQL API introduced in
`soaprs-sqlx 0.2`.

## Current scope

- PostgreSQL logical-field allow-list,
- portable `Condition` and `FindParams` compiler,
- ordered typed bindings using PostgreSQL placeholders,
- explicit null ordering and portable empty-set rewrites,
- SQLx error mapping with preserved technical sources,
- adapter-owned entity mappings and row codecs,
- generic PostgreSQL `ReadRepository` and `WriteRepository`,
- swappable `PgSource` execution boundary with a `PgPoolSource` implementation,
- UUID, JSONB, decimal, date, time, timestamp, and timestamptz persistence,
- schema-qualified tables and per-field select/insert/update permissions,
- PostgreSQL defaults, generated fields, and write operations with `RETURNING`,
- shared transactions and explicit primary/replica routing,
- infrastructure-owned native named-query handlers,
- shared CRUD and complete M1 query-contract coverage,
- PostgreSQL integration tests for database and pool failures.

`PgSource` is intentionally PostgreSQL-specific. Routing between PostgreSQL,
MongoDB, an HTTP service, or an entity cache belongs above adapters as a type
implementing the neutral `ReadRepository`/`WriteRepository` ports. This keeps
backend capabilities out of a lowest-common-denominator source interface.

## Compatibility

- `soaprs` contracts: `0.6`
- SQLx: `0.9`
- Rust: `1.94` or newer, following the SQLx 0.9 MSRV
- PostgreSQL: 13 or newer

The default feature set enables PostgreSQL and Tokio with rustls/WebPKI TLS:

```toml
[dependencies]
soaprs-sqlx = "0.6"
```

The `postgres-types` default feature enables SQLx support for UUID, chrono,
JSON, and `BigDecimal` values. Disable default features when a smaller or
different runtime/TLS composition is required.

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

## Entity mapping and values

`PgEntityMapping` separates logical application fields from physical columns
and controls which statements include each field:

```rust,ignore
let mapping = PgEntityMapping::in_schema("accounts", "users", "id")?
    .with_immutable_field("id", "id", PgScalarKind::Uuid)?
    .with_field("email", "email_address", PgScalarKind::Text)?
    .with_field("profile", "profile", PgScalarKind::Json)?
    .with_generated_field("created_at", "created_at", PgScalarKind::TimestampTz)?;
```

An infrastructure codec returns `PgValue`, while portable repository filters
continue to use `ScalarValue`. `PgValue::Default` emits SQL `DEFAULT` without a
binding. `insert_returning` and `replace_returning` decode database defaults,
triggers, and generated columns back into an entity.

## Transactions

One transaction source can be injected into multiple repositories:

```rust,ignore
let transaction = PgTransactionSource::begin(&pool).await?;
let source: Arc<dyn PgSource> = transaction.clone();

let users = PgRepository::new(source.clone(), user_codec)?;
let orders = PgRepository::new(source, order_codec)?;

users.insert(user).await?;
orders.insert(order).await?;
transaction.commit().await?;
```

Operations are serialized on the transaction connection. After `commit` or
`rollback`, the source rejects further work.

## Native named queries

Application query types remain SQL-free. Infrastructure implements
`PgNativeQuerySpec<Q>` with trusted static SQL, typed bindings, and row decoding,
then uses `PgNativeQueryHandler<Q>` as the application's `QueryHandler<Q>`.
This covers joins, CTEs, window functions, aggregates, and projections without
adding dedicated repository methods.

## Primary and replicas

`PgPrimaryReplicaSource` round-robins reads across configured replicas and sends
`apply` plus write-`RETURNING` operations to the primary. It deliberately does
not retry or fail over operations: retry belongs to an explicit policy that can
account for idempotency and ambiguous write outcomes.

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
- Cache and routing across different technologies belong above adapters as
  neutral repository decorators, not inside the PostgreSQL source contract.
