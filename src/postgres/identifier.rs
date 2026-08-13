use std::fmt;

use soaprs_core::{SoapError, SoapResult};

/// Validated single PostgreSQL identifier such as a table or column name.
///
/// It deliberately excludes qualification and SQL expressions. Schemas,
/// tables, aliases, and columns are represented as separate trusted values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PgIdentifier(String);

impl PgIdentifier {
    /// Validates a conservative portable subset of PostgreSQL identifiers.
    pub fn new(value: impl Into<String>) -> SoapResult<Self> {
        let value = value.into();
        let mut characters = value.chars();
        let Some(first) = characters.next() else {
            return Err(SoapError::validation(
                "PostgreSQL identifier cannot be empty",
            ));
        };

        let valid = (first == '_' || first.is_ascii_alphabetic())
            && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());

        if valid {
            Ok(Self(value))
        } else {
            Err(SoapError::validation(format!(
                "invalid PostgreSQL identifier `{value}`"
            )))
        }
    }

    /// Returns the unquoted identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn quoted(&self) -> String {
        format!("\"{}\"", self.0)
    }
}

impl fmt::Display for PgIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::PgIdentifier;

    #[test]
    fn validates_identifiers_before_quoting() {
        let identifier = PgIdentifier::new("display_name");
        assert_eq!(
            identifier.ok().map(|value| value.as_str().to_owned()),
            Some("display_name".into())
        );
        assert!(PgIdentifier::new("users.name").is_err());
        assert!(PgIdentifier::new("name; DROP TABLE users").is_err());
        assert!(PgIdentifier::new("").is_err());
    }
}
