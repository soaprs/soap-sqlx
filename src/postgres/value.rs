use soaprs_core::{SoapError, SoapResult};
use soaprs_repository::ScalarValue;

#[cfg(feature = "postgres-types")]
use sqlx::types::{
    Decimal, JsonValue, Uuid,
    chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc},
};

/// PostgreSQL persistence value produced by an entity codec.
///
/// Unlike portable [`ScalarValue`], this type represents PostgreSQL-specific
/// values and the `DEFAULT` write marker. Query conditions continue to use
/// `ScalarValue` so application queries remain database-independent.
#[derive(Debug, Clone, PartialEq)]
pub enum PgValue {
    /// Boolean value.
    Bool(bool),
    /// Signed integer value.
    I64(i64),
    /// Unsigned integer value, encoded without precision loss.
    U64(u64),
    /// Finite floating-point value.
    F64(f64),
    /// UTF-8 text value.
    Text(String),
    /// Binary `bytea` value.
    Bytes(Vec<u8>),
    /// PostgreSQL UUID value.
    #[cfg(feature = "postgres-types")]
    Uuid(Uuid),
    /// PostgreSQL `jsonb` value.
    #[cfg(feature = "postgres-types")]
    Json(JsonValue),
    /// PostgreSQL arbitrary-precision numeric value.
    #[cfg(feature = "postgres-types")]
    Decimal(Decimal),
    /// PostgreSQL `date` value.
    #[cfg(feature = "postgres-types")]
    Date(NaiveDate),
    /// PostgreSQL time without time zone.
    #[cfg(feature = "postgres-types")]
    Time(NaiveTime),
    /// PostgreSQL timestamp without time zone.
    #[cfg(feature = "postgres-types")]
    Timestamp(NaiveDateTime),
    /// PostgreSQL timestamp with time zone, normalized to UTC.
    #[cfg(feature = "postgres-types")]
    TimestampTz(DateTime<Utc>),
    /// Typed SQL null. The mapped column supplies its PostgreSQL type.
    Null,
    /// Uses the column's PostgreSQL `DEFAULT` expression for this write.
    Default,
}

impl PgValue {
    pub(crate) fn validate(&self) -> SoapResult<()> {
        if matches!(self, Self::F64(value) if !value.is_finite()) {
            Err(SoapError::validation(
                "PostgreSQL floating-point persistence value must be finite",
            ))
        } else {
            Ok(())
        }
    }
}

impl TryFrom<ScalarValue> for PgValue {
    type Error = SoapError;

    fn try_from(value: ScalarValue) -> Result<Self, Self::Error> {
        value.validate()?;
        match value {
            ScalarValue::Null => Ok(Self::Null),
            ScalarValue::Bool(value) => Ok(Self::Bool(value)),
            ScalarValue::I64(value) => Ok(Self::I64(value)),
            ScalarValue::U64(value) => Ok(Self::U64(value)),
            ScalarValue::F64(value) => Ok(Self::F64(value)),
            ScalarValue::String(value) => Ok(Self::Text(value)),
            ScalarValue::Bytes(value) => Ok(Self::Bytes(value)),
            ScalarValue::List(_) => Err(SoapError::validation(
                "a PostgreSQL persistence value cannot contain a scalar list",
            )),
        }
    }
}

impl From<bool> for PgValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

macro_rules! signed_value {
    ($($value_type:ty),+ $(,)?) => {
        $(
            impl From<$value_type> for PgValue {
                fn from(value: $value_type) -> Self {
                    Self::I64(i64::from(value))
                }
            }
        )+
    };
}

macro_rules! unsigned_value {
    ($($value_type:ty),+ $(,)?) => {
        $(
            impl From<$value_type> for PgValue {
                fn from(value: $value_type) -> Self {
                    Self::U64(u64::from(value))
                }
            }
        )+
    };
}

signed_value!(i8, i16, i32, i64);
unsigned_value!(u8, u16, u32, u64);

impl From<f32> for PgValue {
    fn from(value: f32) -> Self {
        Self::F64(f64::from(value))
    }
}

impl From<f64> for PgValue {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl From<String> for PgValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for PgValue {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

impl From<Vec<u8>> for PgValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl<T> From<Option<T>> for PgValue
where
    T: Into<Self>,
{
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

#[cfg(feature = "postgres-types")]
impl From<Uuid> for PgValue {
    fn from(value: Uuid) -> Self {
        Self::Uuid(value)
    }
}

#[cfg(feature = "postgres-types")]
impl From<JsonValue> for PgValue {
    fn from(value: JsonValue) -> Self {
        Self::Json(value)
    }
}

#[cfg(feature = "postgres-types")]
impl From<Decimal> for PgValue {
    fn from(value: Decimal) -> Self {
        Self::Decimal(value)
    }
}

#[cfg(feature = "postgres-types")]
impl From<NaiveDate> for PgValue {
    fn from(value: NaiveDate) -> Self {
        Self::Date(value)
    }
}

#[cfg(feature = "postgres-types")]
impl From<NaiveTime> for PgValue {
    fn from(value: NaiveTime) -> Self {
        Self::Time(value)
    }
}

#[cfg(feature = "postgres-types")]
impl From<NaiveDateTime> for PgValue {
    fn from(value: NaiveDateTime) -> Self {
        Self::Timestamp(value)
    }
}

#[cfg(feature = "postgres-types")]
impl From<DateTime<Utc>> for PgValue {
    fn from(value: DateTime<Utc>) -> Self {
        Self::TimestampTz(value)
    }
}

#[cfg(test)]
mod tests {
    use soaprs_core::SoapErrorKind;

    use super::PgValue;

    #[test]
    fn converts_nullable_values_and_rejects_non_finite_floats() {
        assert_eq!(PgValue::from(Some(7_i16)), PgValue::I64(7));
        assert_eq!(PgValue::from(Option::<i16>::None), PgValue::Null);
        assert_eq!(
            PgValue::F64(f64::NAN)
                .validate()
                .as_ref()
                .map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
    }
}
