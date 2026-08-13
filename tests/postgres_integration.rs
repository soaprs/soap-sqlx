#![cfg(feature = "postgres")]

//! PostgreSQL contract tests run explicitly by the integration CI job.

use std::{env, error::Error, sync::Arc, time::Duration};

use soaprs_contract_tests::{
    StandardQueryIds, standard_query_contract, verify_crud_contract, verify_query_contract,
};
use soaprs_core::{Entity, ErrorTransience, SoapError, SoapErrorKind, SoapResult};
use soaprs_repository::{FieldName, FindParams, ReadRepository, ScalarValue, WriteRepository};
use soaprs_sqlx::postgres::{
    PgEntityCodec, PgEntityMapping, PgRepository, PgScalarKind, map_sqlx_error,
};
use sqlx::{PgPool, Row, postgres::PgPoolOptions, postgres::PgRow};

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

    fn value(&self, user: &User, field: &FieldName) -> SoapResult<ScalarValue> {
        match field.as_str() {
            "name" => Ok(user.name.clone().into()),
            "age" => Ok(user.age.into()),
            "active" => Ok(user.active.into()),
            "nickname" => Ok(user
                .nickname
                .clone()
                .map_or(ScalarValue::Null, ScalarValue::from)),
            _ => Err(SoapError::validation(format!(
                "unknown writable user field `{field}`"
            ))),
        }
    }

    fn id_value(&self, id: &u64) -> SoapResult<ScalarValue> {
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
