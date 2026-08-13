#![cfg(feature = "postgres")]

//! PostgreSQL execution smoke tests run explicitly by the integration CI job.

use std::{env, error::Error};

use soaprs_repository::{FieldName, FindParams, SortDirection, Where};
use soaprs_sqlx::postgres::{PgFieldMap, PgQueryCompiler, PgScalarKind};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row};

#[tokio::test]
#[ignore = "requires SOAPRS_POSTGRES_URL"]
async fn compiled_query_executes_against_postgres() -> Result<(), Box<dyn Error>> {
    let database_url = env::var("SOAPRS_POSTGRES_URL")?;
    let pool = PgPool::connect(&database_url).await?;

    sqlx::query(
        "CREATE TEMPORARY TABLE soaprs_users (\
            id BIGINT PRIMARY KEY, \
            display_name TEXT NOT NULL, \
            age NUMERIC NOT NULL, \
            is_active BOOLEAN NOT NULL, \
            nickname TEXT NULL\
        )",
    )
    .execute(&pool)
    .await?;

    for (id, name, age, active, nickname) in [
        (1_i64, "Ada", "36", true, Some("Enchantress")),
        (2_i64, "Linus", "24", false, None),
        (3_i64, "Grace", "42", true, None),
    ] {
        sqlx::query(
            "INSERT INTO soaprs_users (id, display_name, age, is_active, nickname) \
             VALUES ($1, $2, $3::numeric, $4, $5)",
        )
        .bind(id)
        .bind(name)
        .bind(age)
        .bind(active)
        .bind(nickname)
        .execute(&pool)
        .await?;
    }

    let fields = PgFieldMap::new()
        .with("active", "is_active", PgScalarKind::Bool)?
        .with("age", "age", PgScalarKind::Numeric)?;
    let compiler = PgQueryCompiler::new(fields);
    let condition = Where::field("active")?
        .eq(true)
        .and("age")?
        .gte(30_u32)
        .build();
    let params = FindParams::all()
        .matching(condition)
        .sort_by(FieldName::new("age")?, SortDirection::Descending);
    let compiled = compiler.compile_find(&params)?;
    let (fragment, arguments) = compiled.into_sqlx_parts()?;
    let sql = format!("SELECT id FROM soaprs_users {fragment}");

    let rows = sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
        .fetch_all(&pool)
        .await?;
    let ids = rows
        .iter()
        .map(|row| row.try_get::<i64, _>("id"))
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(ids, vec![3, 1]);
    pool.close().await;
    Ok(())
}
