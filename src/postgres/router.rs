use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use soaprs_core::BoxFuture;
use sqlx::{
    Error,
    postgres::{PgArguments, PgQueryResult, PgRow},
};

use super::PgSource;

/// Routes reads across replicas and all writes to the primary source.
///
/// The router performs deterministic round-robin selection and deliberately
/// does not retry or fail over an operation. Callers must choose retry policy
/// with knowledge of idempotency and ambiguous write outcomes.
pub struct PgPrimaryReplicaSource {
    primary: Arc<dyn PgSource>,
    replicas: Vec<Arc<dyn PgSource>>,
    next_replica: AtomicUsize,
}

impl PgPrimaryReplicaSource {
    /// Creates a router. An empty replica list sends reads to the primary.
    pub fn new(primary: Arc<dyn PgSource>, replicas: Vec<Arc<dyn PgSource>>) -> Self {
        Self {
            primary,
            replicas,
            next_replica: AtomicUsize::new(0),
        }
    }

    /// Returns the primary source used for every write.
    pub fn primary(&self) -> &Arc<dyn PgSource> {
        &self.primary
    }

    /// Returns configured read replicas.
    pub fn replicas(&self) -> &[Arc<dyn PgSource>] {
        &self.replicas
    }

    fn read_source(&self) -> &Arc<dyn PgSource> {
        if self.replicas.is_empty() {
            &self.primary
        } else {
            let index = self.next_replica.fetch_add(1, Ordering::Relaxed) % self.replicas.len();
            &self.replicas[index]
        }
    }
}

impl PgSource for PgPrimaryReplicaSource {
    fn fetch_all(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Vec<PgRow>, Error>> {
        self.read_source().fetch_all(sql, arguments)
    }

    fn fetch_optional(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<Option<PgRow>, Error>> {
        self.read_source().fetch_optional(sql, arguments)
    }

    fn fetch_one(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgRow, Error>> {
        self.read_source().fetch_one(sql, arguments)
    }

    fn apply(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgQueryResult, Error>> {
        self.primary.apply(sql, arguments)
    }

    fn apply_one(
        &self,
        sql: String,
        arguments: PgArguments,
    ) -> BoxFuture<'_, Result<PgRow, Error>> {
        self.primary.apply_one(sql, arguments)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use soaprs_core::BoxFuture;
    use sqlx::{
        Error,
        postgres::{PgArguments, PgQueryResult, PgRow},
    };

    use super::PgPrimaryReplicaSource;
    use crate::postgres::PgSource;

    #[derive(Default)]
    struct CountingSource {
        reads: AtomicUsize,
        writes: AtomicUsize,
    }

    impl PgSource for CountingSource {
        fn fetch_all(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<Vec<PgRow>, Error>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn fetch_optional(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<Option<PgRow>, Error>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn fetch_one(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<PgRow, Error>> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn apply(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<PgQueryResult, Error>> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn apply_one(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<PgRow, Error>> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(Error::PoolClosed) })
        }
    }

    #[tokio::test]
    async fn round_robins_reads_and_keeps_writes_on_primary() {
        let primary = Arc::new(CountingSource::default());
        let first_replica = Arc::new(CountingSource::default());
        let second_replica = Arc::new(CountingSource::default());
        let router = PgPrimaryReplicaSource::new(
            primary.clone(),
            vec![first_replica.clone(), second_replica.clone()],
        );

        let _ = router
            .fetch_all("SELECT 1".into(), PgArguments::default())
            .await;
        let _ = router
            .fetch_optional("SELECT 1".into(), PgArguments::default())
            .await;
        let _ = router
            .apply(
                "UPDATE users SET active = true".into(),
                PgArguments::default(),
            )
            .await;
        let _ = router
            .apply_one(
                "INSERT INTO users DEFAULT VALUES RETURNING id".into(),
                PgArguments::default(),
            )
            .await;

        assert_eq!(first_replica.reads.load(Ordering::Relaxed), 1);
        assert_eq!(second_replica.reads.load(Ordering::Relaxed), 1);
        assert_eq!(primary.reads.load(Ordering::Relaxed), 0);
        assert_eq!(primary.writes.load(Ordering::Relaxed), 2);
    }
}
