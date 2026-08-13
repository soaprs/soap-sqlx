use std::{marker::PhantomData, sync::Arc};

use soaprs_core::{BoxFuture, Query, QueryHandler, SoapError, SoapResult};
use sqlx::postgres::{PgArguments, PgRow};

use super::{PgBindValue, PgCompiledQuery, PgScalarKind, PgSource, PgValue, map_sqlx_error};
use crate::postgres::compiler::normalize_entity_binding;

/// Trusted infrastructure-owned SQL and its ordered typed bindings.
///
/// SQL is restricted to a static string, including values produced by
/// `include_str!`. Application query types never carry this statement.
#[derive(Debug, Clone, PartialEq)]
pub struct PgNativeStatement {
    sql: &'static str,
    bindings: Vec<PgBindValue>,
}

impl PgNativeStatement {
    /// Creates a native read statement from trusted static SQL.
    pub const fn new(sql: &'static str) -> Self {
        Self {
            sql,
            bindings: Vec::new(),
        }
    }

    /// Appends a typed binding matching the next PostgreSQL placeholder.
    pub fn bind(mut self, kind: PgScalarKind, value: PgValue) -> SoapResult<Self> {
        if value == PgValue::Default {
            return Err(SoapError::validation(
                "native PostgreSQL query binding cannot use DEFAULT",
            ));
        }
        self.bindings.push(normalize_entity_binding(kind, &value)?);
        Ok(self)
    }

    /// Returns the trusted SQL string.
    pub const fn sql(&self) -> &'static str {
        self.sql
    }

    /// Returns ordered typed bindings for inspection and tests.
    pub fn bindings(&self) -> &[PgBindValue] {
        &self.bindings
    }

    fn into_parts(self) -> SoapResult<(String, PgArguments)> {
        let arguments = PgCompiledQuery::from_bindings(self.bindings).into_arguments()?;
        Ok((self.sql.into(), arguments))
    }
}

/// Infrastructure implementation of one application-level native query.
pub trait PgNativeQuerySpec<Q>: Send + Sync
where
    Q: Query,
{
    /// Compiles domain/application inputs to trusted SQL and typed bindings.
    fn compile(&self, query: Q) -> SoapResult<PgNativeStatement>;

    /// Decodes all returned rows to the output declared by the application query.
    fn decode(&self, rows: &[PgRow]) -> SoapResult<Q::Output>;
}

/// Reusable [`QueryHandler`] for an infrastructure-owned PostgreSQL query spec.
pub struct PgNativeQueryHandler<Q>
where
    Q: Query,
{
    source: Arc<dyn PgSource>,
    spec: Arc<dyn PgNativeQuerySpec<Q>>,
    marker: PhantomData<fn(Q)>,
}

impl<Q> PgNativeQueryHandler<Q>
where
    Q: Query,
{
    /// Creates a handler from a PostgreSQL source and native query spec.
    pub const fn new(source: Arc<dyn PgSource>, spec: Arc<dyn PgNativeQuerySpec<Q>>) -> Self {
        Self {
            source,
            spec,
            marker: PhantomData,
        }
    }

    /// Returns the PostgreSQL execution source.
    pub fn source(&self) -> &Arc<dyn PgSource> {
        &self.source
    }

    /// Returns the infrastructure query specification.
    pub fn spec(&self) -> &Arc<dyn PgNativeQuerySpec<Q>> {
        &self.spec
    }
}

impl<Q> QueryHandler<Q> for PgNativeQueryHandler<Q>
where
    Q: Query + 'static,
{
    fn query(&self, query: Q) -> BoxFuture<'_, SoapResult<Q::Output>> {
        Box::pin(async move {
            let statement = self.spec.compile(query)?;
            let (sql, arguments) = statement.into_parts()?;
            let rows = self
                .source
                .fetch_all(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "run native query"))?;
            self.spec.decode(&rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use soaprs_core::SoapErrorKind;

    use super::PgNativeStatement;
    use crate::postgres::{PgBindValue, PgScalarKind, PgValue};

    #[test]
    fn accepts_only_typed_values_and_rejects_default() {
        let statement = PgNativeStatement::new("SELECT * FROM users WHERE age >= $1::numeric")
            .bind(PgScalarKind::Numeric, PgValue::from(18_u32));
        let invalid =
            PgNativeStatement::new("SELECT 1").bind(PgScalarKind::Numeric, PgValue::Default);

        assert_eq!(
            statement.ok().map(|statement| statement.bindings),
            Some(vec![PgBindValue::Numeric("18".into())])
        );
        assert_eq!(
            invalid.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
    }
}
