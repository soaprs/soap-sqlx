use std::sync::Arc;

use async_lock::Mutex;
use soaprs_core::{BoxFuture, SoapError, SoapResult};
use sqlx::{
    AssertSqlSafe, Error, PgPool, Postgres, Transaction,
    postgres::{PgArguments, PgQueryResult, PgRow},
};

use super::{PgSource, map_sqlx_error};

enum TransactionState {
    Active(Transaction<'static, Postgres>),
    Completed,
}

/// Shared PostgreSQL transaction source for multiple repositories and handlers.
///
/// All operations are serialized through the transaction's single connection.
/// Commit and rollback mark the source as completed; later operations fail
/// rather than silently acquiring a non-transactional connection.
pub struct PgTransactionSource {
    state: Mutex<TransactionState>,
}

impl PgTransactionSource {
    /// Begins a transaction and returns a source shareable through [`Arc`].
    pub async fn begin(pool: &PgPool) -> SoapResult<Arc<Self>> {
        let transaction = pool
            .begin()
            .await
            .map_err(|error| map_sqlx_error(error, "begin transaction"))?;
        Ok(Arc::new(Self {
            state: Mutex::new(TransactionState::Active(transaction)),
        }))
    }

    /// Commits the transaction and permanently completes this source.
    pub async fn commit(&self) -> SoapResult<()> {
        let transaction = self.take_active().await?;
        transaction
            .commit()
            .await
            .map_err(|error| map_sqlx_error(error, "commit transaction"))
    }

    /// Rolls back the transaction and permanently completes this source.
    pub async fn rollback(&self) -> SoapResult<()> {
        let transaction = self.take_active().await?;
        transaction
            .rollback()
            .await
            .map_err(|error| map_sqlx_error(error, "rollback transaction"))
    }

    async fn take_active(&self) -> SoapResult<Transaction<'static, Postgres>> {
        let mut state = self.state.lock().await;
        match std::mem::replace(&mut *state, TransactionState::Completed) {
            TransactionState::Active(transaction) => Ok(transaction),
            TransactionState::Completed => Err(SoapError::infrastructure(
                "PostgreSQL transaction source is already completed",
            )),
        }
    }
}

impl PgSource for PgTransactionSource {
    fn fetch_all(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Vec<PgRow>, Error>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let transaction = active(&mut state)?;
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_all(&mut **transaction)
                .await
        })
    }

    fn fetch_optional(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Option<PgRow>, Error>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let transaction = active(&mut state)?;
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_optional(&mut **transaction)
                .await
        })
    }

    fn fetch_one(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgRow, Error>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let transaction = active(&mut state)?;
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_one(&mut **transaction)
                .await
        })
    }

    fn apply(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgQueryResult, Error>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let transaction = active(&mut state)?;
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .execute(&mut **transaction)
                .await
        })
    }

    fn apply_one(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgRow, Error>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let transaction = active(&mut state)?;
            sqlx::query_with::<Postgres, _>(AssertSqlSafe(sql), arguments)
                .fetch_one(&mut **transaction)
                .await
        })
    }
}

fn active(state: &mut TransactionState) -> Result<&mut Transaction<'static, Postgres>, Error> {
    match state {
        TransactionState::Active(transaction) => Ok(transaction),
        TransactionState::Completed => Err(Error::Protocol(
            "PostgreSQL transaction source is already completed".into(),
        )),
    }
}
