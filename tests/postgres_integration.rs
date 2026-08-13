#![cfg(feature = "postgres")]

//! PostgreSQL contract tests run explicitly by the integration CI job.

use std::{env, error::Error, sync::Arc, time::Duration};

use soaprs_contract_tests::{
    StandardQueryIds, standard_query_contract, verify_crud_contract, verify_query_contract,
};
use soaprs_core::{
    Entity, ErrorTransience, Query, QueryHandler, SoapError, SoapErrorKind, SoapResult,
};
use soaprs_repository::{FieldName, FindParams, ReadRepository, WriteRepository};
use soaprs_sqlx::postgres::{
    PgEntityCodec, PgEntityMapping, PgNativeQueryHandler, PgNativeQuerySpec, PgNativeStatement,
    PgPoolSource, PgRepository, PgScalarKind, PgSource, PgTransactionSource, PgValue,
    map_sqlx_error,
};
use sqlx::{PgPool, Row, postgres::PgPoolOptions, postgres::PgRow};

#[cfg(feature = "postgres-types")]
use sqlx::types::{
    Decimal, JsonValue, Uuid,
    chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc},
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct User {
    id: u64,
    name: String,
    age: u32,
    active: bool,
    nickname: Option<String>,
}

impl User {
    fn new(id: u64, name: &str, age: u32, active: bool) -> Self {
        Self {
            id,
            name: name.into(),
            age,
            active,
            nickname: None,
        }
    }

    fn with_nickname(mut self, nickname: &str) -> Self {
        self.nickname = Some(nickname.into());
        self
    }
}

impl Entity for User {
    type Id = u64;

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

struct UserCodec {
    mapping: PgEntityMapping,
}

impl UserCodec {
    fn new() -> SoapResult<Self> {
        let mapping = PgEntityMapping::new("soaprs_users", "id")?
            .with_field("id", "id", PgScalarKind::Numeric)?
            .with_field("name", "display_name", PgScalarKind::Text)?
            .with_field("age", "age", PgScalarKind::Numeric)?
            .with_field("active", "is_active", PgScalarKind::Bool)?
            .with_field("nickname", "nickname", PgScalarKind::Text)?;
        Ok(Self { mapping })
    }
}

impl PgEntityCodec<User> for UserCodec {
    fn mapping(&self) -> &PgEntityMapping {
        &self.mapping
    }

    fn decode(&self, row: &PgRow) -> SoapResult<User> {
        let id: i64 = row
            .try_get("id")
            .map_err(|error| map_sqlx_error(error, "decode user identifier"))?;
        let age: i64 = row
            .try_get("age")
            .map_err(|error| map_sqlx_error(error, "decode user age"))?;
        Ok(User {
            id: u64::try_from(id)
                .map_err(|_| SoapError::infrastructure("persisted user id is negative"))?,
            name: row
                .try_get("display_name")
                .map_err(|error| map_sqlx_error(error, "decode user name"))?,
            age: u32::try_from(age)
                .map_err(|_| SoapError::infrastructure("persisted user age is out of range"))?,
            active: row
                .try_get("is_active")
                .map_err(|error| map_sqlx_error(error, "decode user status"))?,
            nickname: row
                .try_get("nickname")
                .map_err(|error| map_sqlx_error(error, "decode user nickname"))?,
        })
    }

    fn value(&self, user: &User, field: &FieldName) -> SoapResult<PgValue> {
        match field.as_str() {
            "name" => Ok(user.name.clone().into()),
            "age" => Ok(user.age.into()),
            "active" => Ok(user.active.into()),
            "nickname" => Ok(user.nickname.clone().map_or(PgValue::Null, PgValue::from)),
            _ => Err(SoapError::validation(format!(
                "unknown writable user field `{field}`"
            ))),
        }
    }

    fn id_value(&self, id: &u64) -> SoapResult<PgValue> {
        Ok((*id).into())
    }
}

async fn repository() -> Result<(PgPool, PgRepository<User>), Box<dyn Error>> {
    let database_url = env::var("SOAPRS_POSTGRES_URL")?;
    // A temporary table is scoped to one PostgreSQL connection.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(250))
        .connect(&database_url)
        .await?;
    sqlx::query(
        "CREATE TEMPORARY TABLE soaprs_users (\
            id BIGINT PRIMARY KEY, \
            display_name TEXT NOT NULL, \
            age BIGINT NOT NULL, \
            is_active BOOLEAN NOT NULL, \
            nickname TEXT NULL\
        )",
    )
    .execute(&pool)
    .await?;

    let codec: Arc<dyn PgEntityCodec<User>> = Arc::new(UserCodec::new()?);
    let repository = PgRepository::from_pool(pool.clone(), codec)?;
    Ok((pool, repository))
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn satisfies_the_shared_crud_contract() -> Result<(), Box<dyn Error>> {
    let (pool, repository) = repository().await?;
    verify_crud_contract(
        &repository,
        User::new(1, "Ada", 36, true),
        User::new(1, "Ada Lovelace", 37, true),
        User::new(2, "Grace", 45, true),
    )
    .await?;
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn satisfies_the_complete_query_contract() -> Result<(), Box<dyn Error>> {
    let (pool, repository) = repository().await?;
    let contract = standard_query_contract(StandardQueryIds {
        ada: 1,
        grace: 2,
        linus: 3,
        alan: 4,
    })?;
    verify_query_contract(
        &repository,
        vec![
            User::new(1, "Ada", 36, true),
            User::new(2, "Grace", 45, true).with_nickname("Admiral"),
            User::new(3, "Linus", 24, false),
            User::new(4, "Alan", 45, true).with_nickname("Turing"),
        ],
        contract,
    )
    .await?;
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn maps_a_duplicate_identifier_to_conflict() -> Result<(), Box<dyn Error>> {
    let (pool, repository) = repository().await?;
    repository.insert(User::new(1, "Ada", 36, true)).await?;
    let duplicate = repository
        .insert(User::new(1, "Duplicate", 20, false))
        .await;
    assert_eq!(
        duplicate.map_err(|error| error.kind()),
        Err(SoapErrorKind::Conflict)
    );
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn maps_foreign_key_violations_from_postgres() -> Result<(), Box<dyn Error>> {
    let (pool, _repository) = repository().await?;
    sqlx::query("CREATE TEMPORARY TABLE soaprs_parents (id BIGINT PRIMARY KEY)")
        .execute(&pool)
        .await?;
    sqlx::query(
        "CREATE TEMPORARY TABLE soaprs_children (\
            id BIGINT PRIMARY KEY, \
            parent_id BIGINT NOT NULL REFERENCES soaprs_parents(id)\
        )",
    )
    .execute(&pool)
    .await?;

    let database_error = sqlx::query("INSERT INTO soaprs_children VALUES (1, 404)")
        .execute(&pool)
        .await;
    let mapped = match database_error {
        Ok(_) => return Err("foreign-key violation unexpectedly succeeded".into()),
        Err(error) => map_sqlx_error(error, "insert child"),
    };
    assert_eq!(mapped.kind(), SoapErrorKind::Conflict);
    assert!(mapped.source().is_some());
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn preserves_mapping_failures_as_permanent_infrastructure_errors()
-> Result<(), Box<dyn Error>> {
    let (pool, repository) = repository().await?;
    sqlx::query("ALTER TABLE soaprs_users DROP COLUMN display_name")
        .execute(&pool)
        .await?;

    let result = repository.find(FindParams::all()).await;
    let error = match result {
        Ok(_) => return Err("invalid mapping unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert_eq!(error.kind(), SoapErrorKind::Infrastructure);
    assert_eq!(error.transience(), ErrorTransience::Permanent);
    assert!(error.source().is_some());
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn maps_pool_acquisition_timeout_to_timeout() -> Result<(), Box<dyn Error>> {
    let (pool, repository) = repository().await?;
    let held_connection = pool.acquire().await?;

    let result = repository.count(FindParams::all()).await;
    let error = match result {
        Ok(_) => return Err("pool acquisition unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert_eq!(error.kind(), SoapErrorKind::Timeout);
    assert_eq!(error.transience(), ErrorTransience::Transient);
    assert!(error.source().is_some());

    drop(held_connection);
    pool.close().await;
    Ok(())
}

#[cfg(feature = "postgres-types")]
#[derive(Debug, Clone, PartialEq)]
struct TypedRecord {
    id: Uuid,
    status: String,
    payload: JsonValue,
    amount: Decimal,
    business_date: NaiveDate,
    opens_at: NaiveTime,
    happened_at: NaiveDateTime,
    observed_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

#[cfg(feature = "postgres-types")]
impl Entity for TypedRecord {
    type Id = Uuid;

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

#[cfg(feature = "postgres-types")]
struct TypedRecordCodec {
    mapping: PgEntityMapping,
}

#[cfg(feature = "postgres-types")]
impl TypedRecordCodec {
    fn new() -> SoapResult<Self> {
        Ok(Self {
            mapping: PgEntityMapping::new("soaprs_typed_records", "id")?
                .with_immutable_field("id", "id", PgScalarKind::Uuid)?
                .with_field("status", "status", PgScalarKind::Text)?
                .with_field("payload", "payload", PgScalarKind::Json)?
                .with_field("amount", "amount", PgScalarKind::Numeric)?
                .with_field("business_date", "business_date", PgScalarKind::Date)?
                .with_field("opens_at", "opens_at", PgScalarKind::Time)?
                .with_field("happened_at", "happened_at", PgScalarKind::Timestamp)?
                .with_field("observed_at", "observed_at", PgScalarKind::TimestampTz)?
                .with_generated_field("created_at", "created_at", PgScalarKind::TimestampTz)?,
        })
    }
}

#[cfg(feature = "postgres-types")]
impl PgEntityCodec<TypedRecord> for TypedRecordCodec {
    fn mapping(&self) -> &PgEntityMapping {
        &self.mapping
    }

    fn decode(&self, row: &PgRow) -> SoapResult<TypedRecord> {
        Ok(TypedRecord {
            id: decode_column(row, "id")?,
            status: decode_column(row, "status")?,
            payload: decode_column(row, "payload")?,
            amount: decode_column(row, "amount")?,
            business_date: decode_column(row, "business_date")?,
            opens_at: decode_column(row, "opens_at")?,
            happened_at: decode_column(row, "happened_at")?,
            observed_at: decode_column(row, "observed_at")?,
            created_at: decode_column(row, "created_at")?,
        })
    }

    fn value(&self, record: &TypedRecord, field: &FieldName) -> SoapResult<PgValue> {
        match field.as_str() {
            "status" if record.status.is_empty() => Ok(PgValue::Default),
            "status" => Ok(record.status.clone().into()),
            "payload" => Ok(record.payload.clone().into()),
            "amount" => Ok(record.amount.into()),
            "business_date" => Ok(record.business_date.into()),
            "opens_at" => Ok(record.opens_at.into()),
            "happened_at" => Ok(record.happened_at.into()),
            "observed_at" => Ok(record.observed_at.into()),
            _ => Err(SoapError::validation(format!(
                "unknown writable typed-record field `{field}`"
            ))),
        }
    }

    fn id_value(&self, id: &Uuid) -> SoapResult<PgValue> {
        Ok((*id).into())
    }
}

#[cfg(feature = "postgres-types")]
fn decode_column<T>(row: &PgRow, column: &'static str) -> SoapResult<T>
where
    for<'row> T: sqlx::Decode<'row, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(column)
        .map_err(|error| map_sqlx_error(error, "decode typed record"))
}

#[cfg(feature = "postgres-types")]
#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn persists_native_types_defaults_and_returning_rows() -> Result<(), Box<dyn Error>> {
    let database_url = env::var("SOAPRS_POSTGRES_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;
    sqlx::query(
        "CREATE TEMPORARY TABLE soaprs_typed_records (\
            id UUID PRIMARY KEY, \
            status TEXT NOT NULL DEFAULT 'pending', \
            payload JSONB NOT NULL, \
            amount NUMERIC NOT NULL, \
            business_date DATE NOT NULL, \
            opens_at TIME NOT NULL, \
            happened_at TIMESTAMP NOT NULL, \
            observed_at TIMESTAMPTZ NOT NULL, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT now()\
        )",
    )
    .execute(&pool)
    .await?;

    let codec: Arc<dyn PgEntityCodec<TypedRecord>> = Arc::new(TypedRecordCodec::new()?);
    let repository = PgRepository::from_pool(pool.clone(), codec)?;
    let placeholder_created_at =
        DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")?.with_timezone(&Utc);
    let record = TypedRecord {
        id: Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8")?,
        status: String::new(),
        payload: JsonValue::String("created".into()),
        amount: Decimal::new(12345, 2),
        business_date: NaiveDate::parse_from_str("2026-08-13", "%Y-%m-%d")?,
        opens_at: NaiveTime::parse_from_str("09:30:45", "%H:%M:%S")?,
        happened_at: NaiveDateTime::parse_from_str("2026-08-13 10:15:30", "%Y-%m-%d %H:%M:%S")?,
        observed_at: DateTime::parse_from_rfc3339("2026-08-13T10:15:30+02:00")?.with_timezone(&Utc),
        created_at: placeholder_created_at,
    };

    let inserted = repository.insert_returning(record.clone()).await?;
    assert_eq!(inserted.id, record.id);
    assert_eq!(inserted.status, "pending");
    assert_eq!(inserted.payload, record.payload);
    assert_eq!(inserted.amount, record.amount);
    assert_eq!(inserted.business_date, record.business_date);
    assert_eq!(inserted.opens_at, record.opens_at);
    assert_eq!(inserted.happened_at, record.happened_at);
    assert_eq!(inserted.observed_at, record.observed_at);
    assert_ne!(inserted.created_at, placeholder_created_at);
    assert_eq!(repository.get(&record.id).await?, Some(inserted.clone()));

    let mut replacement = inserted.clone();
    replacement.status = "active".into();
    replacement.payload = JsonValue::String("replaced".into());
    replacement.amount = Decimal::new(999, 1);
    let replaced = repository.replace_returning(replacement.clone()).await?;
    assert_eq!(replaced, replacement);

    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn shares_commit_and_rollback_across_transactional_repositories() -> Result<(), Box<dyn Error>>
{
    let (pool, _pool_repository) = repository().await?;

    let rolled_back = PgTransactionSource::begin(&pool).await?;
    let rollback_source: Arc<dyn PgSource> = rolled_back.clone();
    let rollback_codec: Arc<dyn PgEntityCodec<User>> = Arc::new(UserCodec::new()?);
    let rollback_repository = PgRepository::new(rollback_source, rollback_codec)?;
    rollback_repository
        .insert(User::new(1, "Rolled back", 20, true))
        .await?;
    assert_eq!(rollback_repository.count(FindParams::all()).await?, 1);
    rolled_back.rollback().await?;

    let count_after_rollback: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM soaprs_users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count_after_rollback, 0);
    let completed_error = rollback_repository.count(FindParams::all()).await;
    assert_eq!(
        completed_error.map_err(|error| error.kind()),
        Err(SoapErrorKind::Infrastructure)
    );

    let committed = PgTransactionSource::begin(&pool).await?;
    let commit_source: Arc<dyn PgSource> = committed.clone();
    let commit_codec: Arc<dyn PgEntityCodec<User>> = Arc::new(UserCodec::new()?);
    let commit_repository = PgRepository::new(commit_source, commit_codec)?;
    commit_repository
        .insert(User::new(2, "Committed", 30, true))
        .await?;
    committed.commit().await?;

    let count_after_commit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM soaprs_users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count_after_commit, 1);
    pool.close().await;
    Ok(())
}

struct ActiveNames {
    minimum_age: u32,
}

impl Query for ActiveNames {
    type Output = Vec<String>;
}

struct ActiveNamesSpec;

impl PgNativeQuerySpec<ActiveNames> for ActiveNamesSpec {
    fn compile(&self, query: ActiveNames) -> SoapResult<PgNativeStatement> {
        PgNativeStatement::new(
            "SELECT display_name FROM soaprs_users \
             WHERE is_active = true AND age >= $1::numeric \
             ORDER BY display_name COLLATE \"C\"",
        )
        .bind(PgScalarKind::Numeric, query.minimum_age.into())
    }

    fn decode(&self, rows: &[PgRow]) -> SoapResult<Vec<String>> {
        rows.iter()
            .map(|row| {
                row.try_get("display_name")
                    .map_err(|error| map_sqlx_error(error, "decode active user name"))
            })
            .collect()
    }
}

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn runs_infrastructure_owned_native_named_queries() -> Result<(), Box<dyn Error>> {
    let (pool, repository) = repository().await?;
    repository.insert(User::new(1, "Ada", 36, true)).await?;
    repository.insert(User::new(2, "Grace", 45, true)).await?;
    repository.insert(User::new(3, "Linus", 24, false)).await?;

    let source: Arc<dyn PgSource> = Arc::new(PgPoolSource::new(pool.clone()));
    let spec: Arc<dyn PgNativeQuerySpec<ActiveNames>> = Arc::new(ActiveNamesSpec);
    let handler = PgNativeQueryHandler::new(source, spec);
    let names = handler.query(ActiveNames { minimum_age: 40 }).await?;

    assert_eq!(names, vec!["Grace"]);
    pool.close().await;
    Ok(())
}
