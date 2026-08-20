use std::{error::Error, fmt};

use soaprs_core::{SoapError, SoapResult};
use soaprs_repository::{
    Condition, FindParams, LogicalOperator, Operator, ScalarValue, SortDirection,
};
use sqlx::{Arguments, postgres::PgArguments};

#[cfg(feature = "postgres-types")]
use sqlx::types::{
    BigDecimal, Json, JsonValue, Uuid,
    chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc},
};

use super::{PgColumn, PgFieldMap, PgScalarKind, PgValue};

/// Typed value bound to a compiled PostgreSQL query.
#[derive(Debug, Clone, PartialEq)]
pub enum PgBindValue {
    /// Boolean binding.
    Bool(bool),
    /// Exact decimal text bound and cast to PostgreSQL `numeric`.
    Numeric(String),
    /// UTF-8 text binding.
    Text(String),
    /// Binary `bytea` binding.
    Bytes(Vec<u8>),
    /// Native PostgreSQL UUID binding.
    #[cfg(feature = "postgres-types")]
    Uuid(Uuid),
    /// Native PostgreSQL `jsonb` binding.
    #[cfg(feature = "postgres-types")]
    Json(JsonValue),
    /// Native PostgreSQL arbitrary-precision numeric binding.
    #[cfg(feature = "postgres-types")]
    Decimal(BigDecimal),
    /// Native PostgreSQL date binding.
    #[cfg(feature = "postgres-types")]
    Date(NaiveDate),
    /// Native PostgreSQL time binding.
    #[cfg(feature = "postgres-types")]
    Time(NaiveTime),
    /// Native PostgreSQL timestamp binding.
    #[cfg(feature = "postgres-types")]
    Timestamp(NaiveDateTime),
    /// Native PostgreSQL timestamp-with-time-zone binding.
    #[cfg(feature = "postgres-types")]
    TimestampTz(DateTime<Utc>),
    /// Signed integer used for pagination.
    I64(i64),
    /// Typed SQL null binding.
    Null(PgScalarKind),
}

/// SQL fragment plus ordered typed bindings produced by the compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct PgCompiledQuery {
    sql: String,
    bindings: Vec<PgBindValue>,
}

impl PgCompiledQuery {
    pub(crate) const fn from_bindings(bindings: Vec<PgBindValue>) -> Self {
        Self {
            sql: String::new(),
            bindings,
        }
    }

    /// Returns the compiled SQL fragment.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Returns bindings in PostgreSQL placeholder order.
    pub fn bindings(&self) -> &[PgBindValue] {
        &self.bindings
    }

    /// Splits the compiled fragment into SQL text and inspectable bindings.
    pub fn into_parts(self) -> (String, Vec<PgBindValue>) {
        (self.sql, self.bindings)
    }

    /// Converts typed bindings to SQLx PostgreSQL arguments.
    ///
    /// The SQL text remains separate so repository code can combine this
    /// compiler-owned fragment only with adapter-owned statement text.
    pub fn into_arguments(self) -> SoapResult<PgArguments> {
        let mut arguments = PgArguments::default();
        for binding in self.bindings {
            let result = match binding {
                PgBindValue::Bool(value) => arguments.add(value),
                PgBindValue::Numeric(value) | PgBindValue::Text(value) => arguments.add(value),
                PgBindValue::Bytes(value) => arguments.add(value),
                #[cfg(feature = "postgres-types")]
                PgBindValue::Uuid(value) => arguments.add(value),
                #[cfg(feature = "postgres-types")]
                PgBindValue::Json(value) => arguments.add(Json(value)),
                #[cfg(feature = "postgres-types")]
                PgBindValue::Decimal(value) => arguments.add(value),
                #[cfg(feature = "postgres-types")]
                PgBindValue::Date(value) => arguments.add(value),
                #[cfg(feature = "postgres-types")]
                PgBindValue::Time(value) => arguments.add(value),
                #[cfg(feature = "postgres-types")]
                PgBindValue::Timestamp(value) => arguments.add(value),
                #[cfg(feature = "postgres-types")]
                PgBindValue::TimestampTz(value) => arguments.add(value),
                PgBindValue::I64(value) => arguments.add(value),
                PgBindValue::Null(PgScalarKind::Bool) => arguments.add(Option::<bool>::None),
                PgBindValue::Null(PgScalarKind::Numeric | PgScalarKind::Text) => {
                    arguments.add(Option::<String>::None)
                }
                PgBindValue::Null(PgScalarKind::Bytes) => arguments.add(Option::<Vec<u8>>::None),
                PgBindValue::Null(
                    PgScalarKind::Uuid
                    | PgScalarKind::Json
                    | PgScalarKind::Date
                    | PgScalarKind::Time
                    | PgScalarKind::Timestamp
                    | PgScalarKind::TimestampTz,
                ) => arguments.add(Option::<String>::None),
            };
            result.map_err(|source| {
                SoapError::infrastructure("failed to encode a PostgreSQL query binding")
                    .with_source(PgBindingEncodeError(source))
            })?;
        }
        Ok(arguments)
    }

    /// Converts the compiled result to dynamic SQLx query inputs.
    pub fn into_sqlx_parts(self) -> SoapResult<(String, PgArguments)> {
        let sql = self.sql.clone();
        let arguments = self.into_arguments()?;
        Ok((sql, arguments))
    }
}

#[derive(Debug)]
struct PgBindingEncodeError(Box<dyn Error + Send + Sync + 'static>);

impl fmt::Display for PgBindingEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for PgBindingEncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Compiles portable soaprs query values to PostgreSQL SQL fragments.
#[derive(Debug, Clone)]
pub struct PgQueryCompiler {
    fields: PgFieldMap,
}

impl PgQueryCompiler {
    /// Creates a compiler using an explicit logical-field allow-list.
    pub const fn new(fields: PgFieldMap) -> Self {
        Self { fields }
    }

    /// Compiles one validated condition without a leading `WHERE` keyword.
    pub fn compile_condition(&self, condition: &Condition) -> SoapResult<PgCompiledQuery> {
        condition.validate()?;
        let mut output = CompilerOutput::default();
        self.write_condition(condition, &mut output)?;
        Ok(output.finish())
    }

    /// Compiles filtering, sorting, and pagination for a `SELECT` statement.
    ///
    /// The returned fragment is empty when all parameters use their defaults.
    pub fn compile_find(&self, params: &FindParams) -> SoapResult<PgCompiledQuery> {
        params.validate()?;
        let mut output = CompilerOutput::default();

        if let Some(condition) = &params.condition {
            output.sql.push_str("WHERE ");
            self.write_condition(condition, &mut output)?;
        }

        if !params.sort.is_empty() {
            if !output.sql.is_empty() {
                output.sql.push(' ');
            }
            output.sql.push_str("ORDER BY ");
            for (index, sort) in params.sort.iter().enumerate() {
                if index > 0 {
                    output.sql.push_str(", ");
                }
                let column = self.fields.resolve(&sort.field)?;
                output.sql.push_str(&column_expression(column));
                match sort.direction {
                    SortDirection::Ascending => output.sql.push_str(" ASC NULLS FIRST"),
                    SortDirection::Descending => output.sql.push_str(" DESC NULLS LAST"),
                }
            }
        }

        if let Some(limit) = params.limit {
            separate_clause(&mut output.sql);
            output.sql.push_str("LIMIT ");
            let limit = i64::try_from(limit)
                .map_err(|_| SoapError::validation("PostgreSQL limit exceeds BIGINT range"))?;
            output.push_binding(PgBindValue::I64(limit), None);
        }

        if params.offset > 0 {
            separate_clause(&mut output.sql);
            output.sql.push_str("OFFSET ");
            let offset = i64::try_from(params.offset)
                .map_err(|_| SoapError::validation("PostgreSQL offset exceeds BIGINT range"))?;
            output.push_binding(PgBindValue::I64(offset), None);
        }

        Ok(output.finish())
    }

    /// Compiles only filtering for a `COUNT` statement.
    ///
    /// Sorting, offset, and limit are intentionally ignored by the portable
    /// repository count contract, including unknown fields used only for sort.
    pub fn compile_count(&self, params: &FindParams) -> SoapResult<PgCompiledQuery> {
        params.validate()?;
        let mut output = CompilerOutput::default();
        if let Some(condition) = &params.condition {
            output.sql.push_str("WHERE ");
            self.write_condition(condition, &mut output)?;
        }
        Ok(output.finish())
    }

    fn write_condition(
        &self,
        condition: &Condition,
        output: &mut CompilerOutput,
    ) -> SoapResult<()> {
        match condition {
            Condition::Group {
                operator,
                conditions,
            } => {
                output.sql.push('(');
                let separator = match operator {
                    LogicalOperator::And => " AND ",
                    LogicalOperator::Or => " OR ",
                };
                for (index, condition) in conditions.iter().enumerate() {
                    if index > 0 {
                        output.sql.push_str(separator);
                    }
                    self.write_condition(condition, output)?;
                }
                output.sql.push(')');
                Ok(())
            }
            Condition::Predicate {
                field,
                operator,
                value,
            } => {
                let column = self.fields.resolve(field)?;
                self.write_predicate(column, *operator, value.as_ref(), output)
            }
        }
    }

    fn write_predicate(
        &self,
        column: &PgColumn,
        operator: Operator,
        value: Option<&ScalarValue>,
        output: &mut CompilerOutput,
    ) -> SoapResult<()> {
        if operator == Operator::Like && column.scalar_kind() != PgScalarKind::Text {
            return Err(SoapError::validation(
                "LIKE requires a PostgreSQL text column",
            ));
        }
        let expression = column_expression(column);
        match operator {
            Operator::IsNull => {
                output.sql.push_str(&expression);
                output.sql.push_str(" IS NULL");
                Ok(())
            }
            Operator::IsNotNull => {
                output.sql.push_str(&expression);
                output.sql.push_str(" IS NOT NULL");
                Ok(())
            }
            Operator::In | Operator::NotIn => {
                let Some(ScalarValue::List(values)) = value else {
                    return Err(SoapError::validation("set operator requires a list value"));
                };
                if values.is_empty() {
                    if operator == Operator::In {
                        output.sql.push_str("FALSE");
                    } else {
                        output.sql.push_str(&expression);
                        output.sql.push_str(" IS NOT NULL");
                    }
                    return Ok(());
                }

                output.sql.push_str(&expression);
                if operator == Operator::In {
                    output.sql.push_str(" IN (");
                } else {
                    output.sql.push_str(" NOT IN (");
                }
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.sql.push_str(", ");
                    }
                    let binding = normalize_binding(column.scalar_kind(), value)?;
                    output.push_binding(binding, placeholder_cast(column.scalar_kind()));
                }
                output.sql.push(')');
                Ok(())
            }
            _ => {
                let Some(value) = value else {
                    return Err(SoapError::validation(
                        "comparison operator requires a value",
                    ));
                };
                let binding = normalize_binding(column.scalar_kind(), value)?;
                output.sql.push_str(&expression);
                output.sql.push_str(match operator {
                    Operator::Eq => " = ",
                    Operator::Ne => " <> ",
                    Operator::Gt => " > ",
                    Operator::Gte => " >= ",
                    Operator::Lt => " < ",
                    Operator::Lte => " <= ",
                    Operator::Like => " LIKE ",
                    Operator::In | Operator::NotIn | Operator::IsNull | Operator::IsNotNull => {
                        return Err(SoapError::infrastructure(
                            "unexpected PostgreSQL compiler operator branch",
                        ));
                    }
                });
                output.push_binding(binding, placeholder_cast(column.scalar_kind()));
                if operator == Operator::Like {
                    output.sql.push_str(" ESCAPE ''");
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Default)]
struct CompilerOutput {
    sql: String,
    bindings: Vec<PgBindValue>,
}

impl CompilerOutput {
    fn push_binding(&mut self, binding: PgBindValue, cast: Option<&'static str>) {
        self.bindings.push(binding);
        self.sql.push('$');
        self.sql.push_str(&self.bindings.len().to_string());
        if let Some(cast) = cast {
            self.sql.push_str("::");
            self.sql.push_str(cast);
        }
    }

    fn finish(self) -> PgCompiledQuery {
        PgCompiledQuery {
            sql: self.sql,
            bindings: self.bindings,
        }
    }
}

pub(crate) fn column_expression(column: &PgColumn) -> String {
    let identifier = column.identifier().quoted();
    match column.scalar_kind() {
        PgScalarKind::Numeric => format!("{identifier}::numeric"),
        PgScalarKind::Text => format!("{identifier} COLLATE \"C\""),
        PgScalarKind::Bool
        | PgScalarKind::Bytes
        | PgScalarKind::Uuid
        | PgScalarKind::Json
        | PgScalarKind::Date
        | PgScalarKind::Time
        | PgScalarKind::Timestamp
        | PgScalarKind::TimestampTz => identifier,
    }
}

pub(crate) fn normalize_binding(
    kind: PgScalarKind,
    value: &ScalarValue,
) -> SoapResult<PgBindValue> {
    value.validate()?;
    match (kind, value) {
        (PgScalarKind::Bool, ScalarValue::Bool(value)) => Ok(PgBindValue::Bool(*value)),
        (PgScalarKind::Numeric, ScalarValue::I64(value)) => {
            Ok(PgBindValue::Numeric(value.to_string()))
        }
        (PgScalarKind::Numeric, ScalarValue::U64(value)) => {
            Ok(PgBindValue::Numeric(value.to_string()))
        }
        (PgScalarKind::Numeric, ScalarValue::F64(value)) => {
            Ok(PgBindValue::Numeric(value.to_string()))
        }
        (PgScalarKind::Text, ScalarValue::String(value)) => Ok(PgBindValue::Text(value.clone())),
        (PgScalarKind::Bytes, ScalarValue::Bytes(value)) => Ok(PgBindValue::Bytes(value.clone())),
        (
            PgScalarKind::Uuid
            | PgScalarKind::Json
            | PgScalarKind::Date
            | PgScalarKind::Time
            | PgScalarKind::Timestamp
            | PgScalarKind::TimestampTz,
            ScalarValue::String(value),
        ) => Ok(PgBindValue::Text(value.clone())),
        (_, ScalarValue::Null) => Err(SoapError::validation(
            "use IS NULL or IS NOT NULL for PostgreSQL null values",
        )),
        (_, ScalarValue::List(_)) => Err(SoapError::validation(
            "a nested PostgreSQL binding list is invalid",
        )),
        _ => Err(SoapError::validation(
            "query value is incompatible with the mapped PostgreSQL column",
        )),
    }
}

pub(crate) fn normalize_entity_binding(
    kind: PgScalarKind,
    value: &PgValue,
) -> SoapResult<PgBindValue> {
    value.validate()?;
    match (kind, value) {
        (_, PgValue::Null) => Ok(PgBindValue::Null(kind)),
        (_, PgValue::Default) => Err(SoapError::infrastructure(
            "PostgreSQL DEFAULT cannot be encoded as a query binding",
        )),
        (PgScalarKind::Bool, PgValue::Bool(value)) => Ok(PgBindValue::Bool(*value)),
        (PgScalarKind::Numeric, PgValue::I64(value)) => Ok(PgBindValue::Numeric(value.to_string())),
        (PgScalarKind::Numeric, PgValue::U64(value)) => Ok(PgBindValue::Numeric(value.to_string())),
        (PgScalarKind::Numeric, PgValue::F64(value)) => Ok(PgBindValue::Numeric(value.to_string())),
        (PgScalarKind::Text, PgValue::Text(value)) => Ok(PgBindValue::Text(value.clone())),
        (PgScalarKind::Bytes, PgValue::Bytes(value)) => Ok(PgBindValue::Bytes(value.clone())),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::Uuid, PgValue::Uuid(value)) => Ok(PgBindValue::Uuid(*value)),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::Json, PgValue::Json(value)) => Ok(PgBindValue::Json(value.clone())),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::Numeric, PgValue::Decimal(value)) => Ok(PgBindValue::Decimal(value.clone())),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::Date, PgValue::Date(value)) => Ok(PgBindValue::Date(*value)),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::Time, PgValue::Time(value)) => Ok(PgBindValue::Time(*value)),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::Timestamp, PgValue::Timestamp(value)) => Ok(PgBindValue::Timestamp(*value)),
        #[cfg(feature = "postgres-types")]
        (PgScalarKind::TimestampTz, PgValue::TimestampTz(value)) => {
            Ok(PgBindValue::TimestampTz(*value))
        }
        _ => Err(SoapError::validation(
            "persistence value is incompatible with the mapped PostgreSQL column",
        )),
    }
}

pub(crate) const fn placeholder_cast(kind: PgScalarKind) -> Option<&'static str> {
    match kind {
        PgScalarKind::Bool | PgScalarKind::Text | PgScalarKind::Bytes => None,
        PgScalarKind::Numeric => Some("numeric"),
        PgScalarKind::Uuid => Some("uuid"),
        PgScalarKind::Json => Some("jsonb"),
        PgScalarKind::Date => Some("date"),
        PgScalarKind::Time => Some("time"),
        PgScalarKind::Timestamp => Some("timestamp"),
        PgScalarKind::TimestampTz => Some("timestamptz"),
    }
}

fn separate_clause(sql: &mut String) {
    if !sql.is_empty() {
        sql.push(' ');
    }
}

#[cfg(test)]
mod tests {
    use soaprs_core::SoapErrorKind;
    use soaprs_repository::{FieldName, FindParams, SortDirection, Where};

    use super::{PgBindValue, PgQueryCompiler};
    use crate::postgres::{PgFieldMap, PgScalarKind};

    fn compiler() -> PgQueryCompiler {
        let mapping = PgFieldMap::new()
            .with("active", "is_active", PgScalarKind::Bool)
            .and_then(|mapping| mapping.with("age", "age", PgScalarKind::Numeric))
            .and_then(|mapping| mapping.with("name", "display_name", PgScalarKind::Text))
            .and_then(|mapping| mapping.with("nickname", "nickname", PgScalarKind::Text));
        match mapping {
            Ok(mapping) => PgQueryCompiler::new(mapping),
            Err(error) => panic!("valid mapping failed: {error}"),
        }
    }

    #[test]
    fn compiles_nested_conditions_and_binding_order() {
        let age_or_name = Where::any(vec![
            Where::field("age")
                .map(|field| field.gte(18_u32).build())
                .unwrap_or_else(|error| panic!("valid age field failed: {error}")),
            Where::field("name")
                .map(|field| field.like(r"A\_%").build())
                .unwrap_or_else(|error| panic!("valid name field failed: {error}")),
        ]);
        let condition = age_or_name.and_then(|group| {
            Where::all(vec![
                Where::field("active")?.eq(true).build(),
                group.build(),
            ])
            .map(Where::build)
        });
        let compiled = condition.and_then(|condition| compiler().compile_condition(&condition));
        let compiled = match compiled {
            Ok(compiled) => compiled,
            Err(error) => panic!("valid condition failed: {error}"),
        };

        assert_eq!(
            compiled.sql(),
            r#"("is_active" = $1 AND ("age"::numeric >= $2::numeric OR "display_name" COLLATE "C" LIKE $3 ESCAPE ''))"#
        );
        assert_eq!(
            compiled.bindings(),
            &[
                PgBindValue::Bool(true),
                PgBindValue::Numeric("18".into()),
                PgBindValue::Text(r"A\_%".into()),
            ]
        );
    }

    #[test]
    fn keeps_untrusted_values_out_of_sql_text() {
        let malicious = "Ada'; DROP TABLE users; --";
        let condition = Where::field("name").map(|field| field.eq(malicious).build());
        let compiled = condition.and_then(|condition| compiler().compile_condition(&condition));
        let compiled = match compiled {
            Ok(compiled) => compiled,
            Err(error) => panic!("valid condition failed: {error}"),
        };

        assert_eq!(compiled.sql(), r#""display_name" COLLATE "C" = $1"#);
        assert!(!compiled.sql().contains(malicious));
        assert_eq!(compiled.bindings(), &[PgBindValue::Text(malicious.into())]);
    }

    #[test]
    fn rewrites_empty_sets_with_portable_null_semantics() {
        let in_empty = Where::field("nickname")
            .map(|field| field.in_values(Vec::<String>::new()).build())
            .and_then(|condition| compiler().compile_condition(&condition));
        let not_in_empty = Where::field("nickname")
            .map(|field| field.not_in_values(Vec::<String>::new()).build())
            .and_then(|condition| compiler().compile_condition(&condition));

        assert_eq!(
            in_empty.ok().map(|query| query.sql().to_owned()),
            Some("FALSE".into())
        );
        assert_eq!(
            not_in_empty.ok().map(|query| query.sql().to_owned()),
            Some(r#""nickname" COLLATE "C" IS NOT NULL"#.into())
        );
    }

    #[test]
    fn emits_explicit_null_ordering_and_bound_pagination() {
        let age = FieldName::new("age");
        let params = age.map(|age| {
            FindParams::all()
                .sort_by(age, SortDirection::Descending)
                .limit(20)
                .offset(40)
        });
        let compiled = params.and_then(|params| compiler().compile_find(&params));
        let compiled = match compiled {
            Ok(compiled) => compiled,
            Err(error) => panic!("valid find params failed: {error}"),
        };

        assert_eq!(
            compiled.sql(),
            r#"ORDER BY "age"::numeric DESC NULLS LAST LIMIT $1 OFFSET $2"#
        );
        assert_eq!(
            compiled.bindings(),
            &[PgBindValue::I64(20), PgBindValue::I64(40)]
        );
    }

    #[test]
    fn rejects_unknown_fields_and_incompatible_values() {
        let unknown = Where::field("missing")
            .map(|field| field.eq(1).build())
            .and_then(|condition| compiler().compile_condition(&condition));
        let incompatible = Where::field("active")
            .map(|field| field.eq("yes").build())
            .and_then(|condition| compiler().compile_condition(&condition));

        assert_eq!(
            unknown.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
        assert_eq!(
            incompatible.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
    }

    #[test]
    fn casts_portable_strings_for_native_columns_and_rejects_like() {
        let fields = PgFieldMap::new().with("id", "id", PgScalarKind::Uuid);
        let compiler = match fields {
            Ok(fields) => PgQueryCompiler::new(fields),
            Err(error) => panic!("valid UUID mapping failed: {error}"),
        };
        let equality = Where::field("id")
            .map(|field| field.eq("67e55044-10b1-426f-9247-bb680e5fe0c8").build())
            .and_then(|condition| compiler.compile_condition(&condition));
        let like = Where::field("id")
            .map(|field| field.like("67e5%").build())
            .and_then(|condition| compiler.compile_condition(&condition));

        assert_eq!(
            equality.ok().map(|statement| statement.sql().to_owned()),
            Some(r#""id" = $1::uuid"#.into())
        );
        assert_eq!(
            like.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
    }

    #[test]
    fn preserves_full_unsigned_integer_precision_through_numeric_text() {
        let condition = Where::field("age")
            .map(|field| field.eq(u64::MAX).build())
            .and_then(|condition| compiler().compile_condition(&condition));
        let compiled = match condition {
            Ok(compiled) => compiled,
            Err(error) => panic!("valid numeric query failed: {error}"),
        };

        assert_eq!(compiled.sql(), r#""age"::numeric = $1::numeric"#);
        assert_eq!(
            compiled.bindings(),
            &[PgBindValue::Numeric(u64::MAX.to_string())]
        );
    }

    #[test]
    fn count_ignores_sort_pagination_and_unknown_sort_fields() {
        let unknown = FieldName::new("not_mapped");
        let params = unknown.map(|unknown| {
            FindParams::all()
                .sort_by(unknown, SortDirection::Ascending)
                .limit(1)
                .offset(2)
        });
        let compiled = params.and_then(|params| compiler().compile_count(&params));
        let compiled = match compiled {
            Ok(compiled) => compiled,
            Err(error) => panic!("valid count params failed: {error}"),
        };

        assert_eq!(compiled.sql(), "");
        assert!(compiled.bindings().is_empty());
    }
}
