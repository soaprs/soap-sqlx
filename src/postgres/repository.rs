use std::{marker::PhantomData, sync::Arc};

use soaprs_core::{BoxFuture, Entity, SoapError, SoapResult};
use soaprs_repository::{Condition, FindParams, Operator, ReadRepository, WriteRepository};
use sqlx::{PgPool, Row};

use super::{
    PgBindValue, PgCompiledQuery, PgEntityCodec, PgPoolSource, PgQueryCompiler, PgSource,
    map_sqlx_error,
};
use crate::postgres::compiler::normalize_entity_binding;

/// Generic PostgreSQL implementation of the soaprs repository ports.
pub struct PgRepository<E>
where
    E: Entity,
{
    source: Arc<dyn PgSource>,
    codec: Arc<dyn PgEntityCodec<E>>,
    compiler: PgQueryCompiler,
    marker: PhantomData<fn() -> E>,
}

impl<E> PgRepository<E>
where
    E: Entity,
{
    /// Creates a repository from an execution source and adapter-owned codec.
    pub fn new(source: Arc<dyn PgSource>, codec: Arc<dyn PgEntityCodec<E>>) -> SoapResult<Self> {
        codec.mapping().validate()?;
        let compiler = PgQueryCompiler::new(codec.mapping().fields().clone());
        Ok(Self {
            source,
            codec,
            compiler,
            marker: PhantomData,
        })
    }

    /// Creates a repository directly from a pool using [`PgPoolSource`].
    pub fn from_pool(pool: PgPool, codec: Arc<dyn PgEntityCodec<E>>) -> SoapResult<Self> {
        let source: Arc<dyn PgSource> = Arc::new(PgPoolSource::new(pool));
        Self::new(source, codec)
    }

    /// Returns the PostgreSQL execution source.
    pub fn source(&self) -> &Arc<dyn PgSource> {
        &self.source
    }

    /// Returns the entity codec used by this repository.
    pub fn codec(&self) -> &Arc<dyn PgEntityCodec<E>> {
        &self.codec
    }

    fn select_prefix(&self) -> SoapResult<String> {
        let mapping = self.codec.mapping();
        let columns = mapping
            .ordered_fields()
            .iter()
            .map(|field| {
                mapping
                    .fields()
                    .resolve(field)
                    .map(|column| column.identifier().quoted())
            })
            .collect::<SoapResult<Vec<_>>>()?
            .join(", ");
        Ok(format!(
            "SELECT {columns} FROM {}",
            mapping.table().quoted()
        ))
    }

    fn compiled_id(&self, id: &E::Id) -> SoapResult<PgCompiledQuery> {
        let value = self.codec.id_value(id)?;
        let condition = Condition::Predicate {
            field: self.codec.mapping().id_field().clone(),
            operator: Operator::Eq,
            value: Some(value),
        };
        self.compiler.compile_condition(&condition)
    }

    fn entity_bindings(&self, entity: &E, include_id: bool) -> SoapResult<Vec<PgBindValue>> {
        let mapping = self.codec.mapping();
        mapping
            .ordered_fields()
            .iter()
            .filter(|field| include_id || *field != mapping.id_field())
            .map(|field| {
                let value = if field == mapping.id_field() {
                    self.codec.id_value(entity.id())?
                } else {
                    self.codec.value(entity, field)?
                };
                let column = mapping.fields().resolve(field)?;
                normalize_entity_binding(column.scalar_kind(), &value)
            })
            .collect()
    }
}

impl<E> ReadRepository<E> for PgRepository<E>
where
    E: Entity + 'static,
{
    fn find(&self, params: FindParams) -> BoxFuture<'_, SoapResult<Vec<E>>> {
        Box::pin(async move {
            let compiled = self.compiler.compile_find(&params)?;
            let (fragment, arguments) = compiled.into_sqlx_parts()?;
            let sql = append_fragment(self.select_prefix()?, &fragment);
            let rows = self
                .source
                .fetch_all(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "find entities"))?;
            rows.iter().map(|row| self.codec.decode(row)).collect()
        })
    }

    fn get<'a>(&'a self, id: &'a E::Id) -> BoxFuture<'a, SoapResult<Option<E>>> {
        Box::pin(async move {
            let compiled = self.compiled_id(id)?;
            let (condition, arguments) = compiled.into_sqlx_parts()?;
            let sql = format!("{} WHERE {condition}", self.select_prefix()?);
            let row = self
                .source
                .fetch_optional(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "get entity"))?;
            row.as_ref().map(|row| self.codec.decode(row)).transpose()
        })
    }

    fn count(&self, params: FindParams) -> BoxFuture<'_, SoapResult<u64>> {
        Box::pin(async move {
            let compiled = self.compiler.compile_count(&params)?;
            let (fragment, arguments) = compiled.into_sqlx_parts()?;
            let sql = append_fragment(
                format!(
                    "SELECT COUNT(*) AS count FROM {}",
                    self.codec.mapping().table().quoted()
                ),
                &fragment,
            );
            let row = self
                .source
                .fetch_one(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "count entities"))?;
            let count: i64 = row
                .try_get("count")
                .map_err(|error| map_sqlx_error(error, "decode entity count"))?;
            u64::try_from(count)
                .map_err(|_| SoapError::infrastructure("PostgreSQL returned a negative count"))
        })
    }
}

impl<E> WriteRepository<E> for PgRepository<E>
where
    E: Entity + 'static,
{
    fn insert(&self, entity: E) -> BoxFuture<'_, SoapResult<()>> {
        Box::pin(async move {
            let mapping = self.codec.mapping();
            let bindings = self.entity_bindings(&entity, true)?;
            let columns = mapping
                .ordered_fields()
                .iter()
                .map(|field| {
                    mapping
                        .fields()
                        .resolve(field)
                        .map(|column| column.identifier().quoted())
                })
                .collect::<SoapResult<Vec<_>>>()?
                .join(", ");
            let placeholders = placeholders_for(mapping, mapping.ordered_fields(), 1)?;
            let sql = format!(
                "INSERT INTO {} ({columns}) VALUES ({placeholders})",
                mapping.table().quoted()
            );
            let arguments = compiled_arguments(bindings)?;
            self.source
                .apply(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "insert entity"))?;
            Ok(())
        })
    }

    fn replace(&self, entity: E) -> BoxFuture<'_, SoapResult<()>> {
        Box::pin(async move {
            let mapping = self.codec.mapping();
            let writable = mapping
                .ordered_fields()
                .iter()
                .filter(|field| *field != mapping.id_field())
                .cloned()
                .collect::<Vec<_>>();
            if writable.is_empty() {
                return Err(SoapError::unsupported(
                    "PostgreSQL replacement requires a non-identity field",
                ));
            }
            let mut bindings = self.entity_bindings(&entity, false)?;
            let id_value = self.codec.id_value(entity.id())?;
            let id_column = mapping.fields().resolve(mapping.id_field())?;
            bindings.push(normalize_entity_binding(
                id_column.scalar_kind(),
                &id_value,
            )?);

            let assignments = writable
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let column = mapping.fields().resolve(field)?;
                    Ok(format!(
                        "{} = {}",
                        column.identifier().quoted(),
                        placeholder(index + 1, column.scalar_kind())
                    ))
                })
                .collect::<SoapResult<Vec<_>>>()?
                .join(", ");
            let id_placeholder = placeholder(writable.len() + 1, id_column.scalar_kind());
            let sql = format!(
                "UPDATE {} SET {assignments} WHERE {} = {id_placeholder}",
                mapping.table().quoted(),
                id_column.identifier().quoted()
            );
            let arguments = compiled_arguments(bindings)?;
            let result = self
                .source
                .apply(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "replace entity"))?;
            if result.rows_affected() == 0 {
                return Err(SoapError::not_found("entity identifier"));
            }
            Ok(())
        })
    }

    fn remove<'a>(&'a self, id: &'a E::Id) -> BoxFuture<'a, SoapResult<bool>> {
        Box::pin(async move {
            let compiled = self.compiled_id(id)?;
            let (condition, arguments) = compiled.into_sqlx_parts()?;
            let sql = format!(
                "DELETE FROM {} WHERE {condition}",
                self.codec.mapping().table().quoted()
            );
            let result = self
                .source
                .apply(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "remove entity"))?;
            Ok(result.rows_affected() > 0)
        })
    }
}

fn append_fragment(mut sql: String, fragment: &str) -> String {
    if !fragment.is_empty() {
        sql.push(' ');
        sql.push_str(fragment);
    }
    sql
}

fn placeholders_for(
    mapping: &super::PgEntityMapping,
    fields: &[soaprs_repository::FieldName],
    first: usize,
) -> SoapResult<String> {
    fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            mapping
                .fields()
                .resolve(field)
                .map(|column| placeholder(first + index, column.scalar_kind()))
        })
        .collect::<SoapResult<Vec<_>>>()
        .map(|items| items.join(", "))
}

fn placeholder(index: usize, kind: super::PgScalarKind) -> String {
    if kind == super::PgScalarKind::Numeric {
        format!("${index}::numeric")
    } else {
        format!("${index}")
    }
}

fn compiled_arguments(bindings: Vec<PgBindValue>) -> SoapResult<sqlx::postgres::PgArguments> {
    PgCompiledQuery::from_bindings(bindings).into_arguments()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use soaprs_core::{BoxFuture, Entity, SoapError, SoapResult};
    use soaprs_repository::{FieldName, ScalarValue, WriteRepository};
    use sqlx::{Error, postgres::PgArguments, postgres::PgQueryResult, postgres::PgRow};

    use super::PgRepository;
    use crate::postgres::{PgEntityCodec, PgEntityMapping, PgScalarKind, PgSource};

    #[derive(Debug)]
    struct User {
        id: u64,
        name: String,
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
            Ok(Self {
                mapping: PgEntityMapping::new("users", "id")?
                    .with_field("id", "id", PgScalarKind::Numeric)?
                    .with_field("name", "display_name", PgScalarKind::Text)?,
            })
        }
    }

    impl PgEntityCodec<User> for UserCodec {
        fn mapping(&self) -> &PgEntityMapping {
            &self.mapping
        }

        fn decode(&self, _row: &PgRow) -> SoapResult<User> {
            Err(SoapError::unsupported(
                "recording source does not return rows",
            ))
        }

        fn value(&self, entity: &User, field: &FieldName) -> SoapResult<ScalarValue> {
            match field.as_str() {
                "name" => Ok(entity.name.clone().into()),
                _ => Err(SoapError::validation("unknown user field")),
            }
        }

        fn id_value(&self, id: &u64) -> SoapResult<ScalarValue> {
            Ok((*id).into())
        }
    }

    #[derive(Default)]
    struct RecordingSource {
        statements: Mutex<Vec<String>>,
    }

    impl PgSource for RecordingSource {
        fn fetch_all(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<Vec<PgRow>, Error>> {
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn fetch_optional(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<Option<PgRow>, Error>> {
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn fetch_one(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<PgRow, Error>> {
            Box::pin(async { Err(Error::PoolClosed) })
        }

        fn apply(
            &self,
            sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<PgQueryResult, Error>> {
            Box::pin(async move {
                self.statements
                    .lock()
                    .map_err(|_| Error::Protocol("recording source lock poisoned".into()))?
                    .push(sql);
                Ok(PgQueryResult::default())
            })
        }
    }

    #[tokio::test]
    async fn delegates_statement_execution_to_the_injected_source() {
        let source = Arc::new(RecordingSource::default());
        let erased_source: Arc<dyn PgSource> = source.clone();
        let codec = UserCodec::new().map(|codec| {
            let codec: Arc<dyn PgEntityCodec<User>> = Arc::new(codec);
            codec
        });
        let repository = codec.and_then(|codec| PgRepository::new(erased_source, codec));
        let result = match repository {
            Ok(repository) => {
                repository
                    .insert(User {
                        id: 7,
                        name: "Ada".into(),
                    })
                    .await
            }
            Err(error) => Err(error),
        };

        assert!(result.is_ok(), "source-backed insert failed: {result:?}");
        let statements = source.statements.lock();
        match statements {
            Ok(statements) => assert_eq!(
                statements.as_slice(),
                [r#"INSERT INTO "users" ("id", "display_name") VALUES ($1::numeric, $2)"#]
            ),
            Err(_) => panic!("recording source lock poisoned"),
        }
    }
}
