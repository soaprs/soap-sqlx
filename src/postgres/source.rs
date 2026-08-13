use soaprs_core::BoxFuture;
use sqlx::{
    AssertSqlSafe, Error, PgPool, Postgres,
    postgres::{PgArguments, PgQueryResult, PgRow},
};

/// PostgreSQL execution boundary used by [`super::PgRepository`].
///
/// A source owns connection selection and statement execution. Repositories
/// own entity mapping and SQL compilation. Keeping those responsibilities
/// separate allows the same repository to run through a pool, transaction,
/// primary/replica router, or instrumented source.
pub trait PgSource: Send + Sync {
    /// Fetches all rows produced by one repository-owned statement.
    fn fetch_all(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Vec<PgRow>, Error>>;

    /// Fetches at most one row produced by one repository-owned statement.
    fn fetch_optional(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Option<PgRow>, Error>>;

    /// Fetches exactly one row produced by one repository-owned statement.
    fn fetch_one(&self, sql: String, arguments: PgArguments)
    -> BoxFuture<'_, Result<PgRow, Error>>;

    /// Executes one repository-owned statement and returns its write result.
    fn apply(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgQueryResult, Error>>;
}

/// [`PgSource`] backed by a SQLx PostgreSQL connection pool.
#[derive(Debug, Clone)]
pub struct PgPoolSource {
    pool: PgPool,
}

impl PgPoolSource {
    /// Creates a source using the supplied pool.
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns the underlying SQLx pool.
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl PgSource for PgPoolSource {
    fn fetch_all(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Vec<PgRow>, Error>> {
        Box::pin(async move {
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_all(&self.pool)
                .await
        })
    }

    fn fetch_optional(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Option<PgRow>, Error>> {
        Box::pin(async move {
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_optional(&self.pool)
                .await
        })
    }

    fn fetch_one(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgRow, Error>> {
        Box::pin(async move {
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_one(&self.pool)
                .await
        })
    }

    fn apply(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgQueryResult, Error>> {
        Box::pin(async move {
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .execute(&self.pool)
                .await
        })
    }
}
