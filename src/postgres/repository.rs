use std::{marker::PhantomData, sync::Arc};

use soaprs_core::{BoxFuture, Entity, SoapError, SoapResult};
use soaprs_repository::{FindParams, ReadRepository, WriteRepository};
use sqlx::{PgPool, Row, postgres::PgArguments};

use super::{
    PgBindValue, PgCompiledQuery, PgEntityCodec, PgPoolSource, PgQueryCompiler, PgScalarKind,
    PgSource, PgValue, map_sqlx_error,
};
use crate::postgres::compiler::{normalize_entity_binding, placeholder_cast};

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

    fn selected_columns(&self) -> SoapResult<String> {
        let mapping = self.codec.mapping();
        mapping
            .selectable_fields()
            .map(|field| {
                mapping
                    .fields()
                    .resolve(field)
                    .map(|column| column.identifier().quoted())
            })
            .collect::<SoapResult<Vec<_>>>()
            .map(|columns| columns.join(", "))
    }

    fn select_prefix(&self) -> SoapResult<String> {
        Ok(format!(
            "SELECT {} FROM {}",
            self.selected_columns()?,
            self.codec.mapping().table().quoted()
        ))
    }

    fn entity_value(
        &self,
        entity: &E,
        field: &soaprs_repository::FieldName,
    ) -> SoapResult<PgValue> {
        if field == self.codec.mapping().id_field() {
            self.codec.id_value(entity.id())
        } else {
            self.codec.value(entity, field)
        }
    }

    fn compiled_id(&self, id: &E::Id) -> SoapResult<(String, PgArguments)> {
        let mapping = self.codec.mapping();
        let column = mapping.fields().resolve(mapping.id_field())?;
        let value = self.codec.id_value(id)?;
        if matches!(value, PgValue::Null | PgValue::Default) {
            return Err(SoapError::validation(
                "PostgreSQL entity identifier cannot be null or default",
            ));
        }
        let binding = normalize_entity_binding(column.scalar_kind(), &value)?;
        let sql = format!(
            "{} = {}",
            column.identifier().quoted(),
            placeholder(1, column.scalar_kind())
        );
        Ok((sql, compiled_arguments(vec![binding])?))
    }

    fn compile_insert(&self, entity: &E, returning: bool) -> SoapResult<(String, PgArguments)> {
        let mapping = self.codec.mapping();
        let fields = mapping.insertable_fields().collect::<Vec<_>>();
        let mut bindings = Vec::new();
        let mut sql = if fields.is_empty() {
            format!("INSERT INTO {} DEFAULT VALUES", mapping.table().quoted())
        } else {
            let columns = fields
                .iter()
                .map(|field| {
                    mapping
                        .fields()
                        .resolve(field)
                        .map(|column| column.identifier().quoted())
                })
                .collect::<SoapResult<Vec<_>>>()?
                .join(", ");
            let values = fields
                .iter()
                .map(|field| {
                    let column = mapping.fields().resolve(field)?;
                    let value = self.entity_value(entity, field)?;
                    if value == PgValue::Default {
                        Ok("DEFAULT".into())
                    } else {
                        bindings.push(normalize_entity_binding(column.scalar_kind(), &value)?);
                        Ok(placeholder(bindings.len(), column.scalar_kind()))
                    }
                })
                .collect::<SoapResult<Vec<_>>>()?
                .join(", ");
            format!(
                "INSERT INTO {} ({columns}) VALUES ({values})",
                mapping.table().quoted()
            )
        };
        if returning {
            sql.push_str(" RETURNING ");
            sql.push_str(&self.selected_columns()?);
        }
        Ok((sql, compiled_arguments(bindings)?))
    }

    fn compile_replace(&self, entity: &E, returning: bool) -> SoapResult<(String, PgArguments)> {
        let mapping = self.codec.mapping();
        let fields = mapping
            .updatable_fields()
            .filter(|field| *field != mapping.id_field())
            .collect::<Vec<_>>();
        if fields.is_empty() {
            return Err(SoapError::unsupported(
                "PostgreSQL replacement requires an updatable non-identity field",
            ));
        }

        let mut bindings = Vec::new();
        let assignments = fields
            .iter()
            .map(|field| {
                let column = mapping.fields().resolve(field)?;
                let value = self.entity_value(entity, field)?;
                let expression = if value == PgValue::Default {
                    "DEFAULT".into()
                } else {
                    bindings.push(normalize_entity_binding(column.scalar_kind(), &value)?);
                    placeholder(bindings.len(), column.scalar_kind())
                };
                Ok(format!("{} = {expression}", column.identifier().quoted()))
            })
            .collect::<SoapResult<Vec<_>>>()?
            .join(", ");

        let id_column = mapping.fields().resolve(mapping.id_field())?;
        let id_value = self.codec.id_value(entity.id())?;
        if matches!(id_value, PgValue::Null | PgValue::Default) {
            return Err(SoapError::validation(
                "PostgreSQL entity identifier cannot be null or default",
            ));
        }
        bindings.push(normalize_entity_binding(
            id_column.scalar_kind(),
            &id_value,
        )?);
        let id_placeholder = placeholder(bindings.len(), id_column.scalar_kind());
        let mut sql = format!(
            "UPDATE {} SET {assignments} WHERE {} = {id_placeholder}",
            mapping.table().quoted(),
            id_column.identifier().quoted()
        );
        if returning {
            sql.push_str(" RETURNING ");
            sql.push_str(&self.selected_columns()?);
        }
        Ok((sql, compiled_arguments(bindings)?))
    }
}

impl<E> PgRepository<E>
where
    E: Entity + 'static,
{
    /// Inserts an entity and decodes database defaults and generated fields.
    pub fn insert_returning(&self, entity: E) -> BoxFuture<'_, SoapResult<E>> {
        Box::pin(async move {
            let (sql, arguments) = self.compile_insert(&entity, true)?;
            let row = self
                .source
                .apply_one(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "insert and return entity"))?;
            self.codec.decode(&row)
        })
    }

    /// Replaces an entity and decodes database-generated replacement values.
    pub fn replace_returning(&self, entity: E) -> BoxFuture<'_, SoapResult<E>> {
        Box::pin(async move {
            let (sql, arguments) = self.compile_replace(&entity, true)?;
            let row = self
                .source
                .apply_one(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "replace and return entity"))?;
            self.codec.decode(&row)
        })
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
            let (condition, arguments) = self.compiled_id(id)?;
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
            let (sql, arguments) = self.compile_insert(&entity, false)?;
            self.source
                .apply(sql, arguments)
                .await
                .map_err(|error| map_sqlx_error(error, "insert entity"))?;
            Ok(())
        })
    }

    fn replace(&self, entity: E) -> BoxFuture<'_, SoapResult<()>> {
        Box::pin(async move {
            let (sql, arguments) = self.compile_replace(&entity, false)?;
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
            let (condition, arguments) = self.compiled_id(id)?;
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

fn placeholder(index: usize, kind: PgScalarKind) -> String {
    match placeholder_cast(kind) {
        Some(cast) => format!("${index}::{cast}"),
        None => format!("${index}"),
    }
}

fn compiled_arguments(bindings: Vec<PgBindValue>) -> SoapResult<PgArguments> {
    PgCompiledQuery::from_bindings(bindings).into_arguments()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use soaprs_core::{BoxFuture, Entity, SoapError, SoapResult};
    use soaprs_repository::{FieldName, WriteRepository};
    use sqlx::{Error, postgres::PgArguments, postgres::PgQueryResult, postgres::PgRow};

    use super::PgRepository;
    use crate::postgres::{PgEntityCodec, PgEntityMapping, PgScalarKind, PgSource, PgValue};

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
                mapping: PgEntityMapping::in_schema("app", "users", "id")?
                    .with_immutable_field("id", "id", PgScalarKind::Numeric)?
                    .with_field("name", "display_name", PgScalarKind::Text)?
                    .with_generated_field("created_at", "created_at", PgScalarKind::TimestampTz)?,
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

        fn value(&self, entity: &User, field: &FieldName) -> SoapResult<PgValue> {
            match field.as_str() {
                "name" if entity.name.is_empty() => Ok(PgValue::Default),
                "name" => Ok(entity.name.clone().into()),
                _ => Err(SoapError::validation("unknown user field")),
            }
        }

        fn id_value(&self, id: &u64) -> SoapResult<PgValue> {
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

        fn apply_one(
            &self,
            _sql: String,
            _arguments: PgArguments,
        ) -> BoxFuture<'_, Result<PgRow, Error>> {
            Box::pin(async { Err(Error::PoolClosed) })
        }
    }

    #[tokio::test]
    async fn delegates_schema_qualified_insert_to_the_injected_source() {
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
                [r#"INSERT INTO "app"."users" ("id", "display_name") VALUES ($1::numeric, $2)"#]
            ),
            Err(_) => panic!("recording source lock poisoned"),
        }
    }

    #[tokio::test]
    async fn emits_default_and_returning_for_database_managed_values() {
        let source: Arc<dyn PgSource> = Arc::new(RecordingSource::default());
        let codec = UserCodec::new().map(|codec| {
            let codec: Arc<dyn PgEntityCodec<User>> = Arc::new(codec);
            codec
        });
        let repository = codec.and_then(|codec| PgRepository::new(source, codec));
        let statement = repository.and_then(|repository| {
            repository.compile_insert(
                &User {
                    id: 9,
                    name: String::new(),
                },
                true,
            )
        });

        assert_eq!(
            statement.ok().map(|(sql, _)| sql),
            Some(
                r#"INSERT INTO "app"."users" ("id", "display_name") VALUES ($1::numeric, DEFAULT) RETURNING "id", "display_name", "created_at""#.into()
            )
        );
    }
}
