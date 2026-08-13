use std::collections::BTreeMap;

use soaprs_core::{SoapError, SoapResult};
use soaprs_repository::FieldName;

use super::PgIdentifier;

/// Portable scalar family expected by a mapped PostgreSQL column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PgScalarKind {
    /// PostgreSQL boolean values.
    Bool,
    /// Signed, unsigned, or floating-point values compared through `numeric`.
    Numeric,
    /// UTF-8 text values.
    Text,
    /// Binary `bytea` values.
    Bytes,
}

/// Physical PostgreSQL column selected by one logical application field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgColumn {
    identifier: PgIdentifier,
    scalar_kind: PgScalarKind,
}

impl PgColumn {
    /// Defines a physical column and the scalar family used for bindings.
    pub const fn new(identifier: PgIdentifier, scalar_kind: PgScalarKind) -> Self {
        Self {
            identifier,
            scalar_kind,
        }
    }

    /// Returns the physical column identifier.
    pub const fn identifier(&self) -> &PgIdentifier {
        &self.identifier
    }

    /// Returns the scalar family used by the compiler.
    pub const fn scalar_kind(&self) -> PgScalarKind {
        self.scalar_kind
    }
}

/// Explicit allow-list mapping logical query fields to physical columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PgFieldMap {
    columns: BTreeMap<FieldName, PgColumn>,
}

impl PgFieldMap {
    /// Creates an empty field mapping.
    pub const fn new() -> Self {
        Self {
            columns: BTreeMap::new(),
        }
    }

    /// Adds one logical-to-physical mapping.
    pub fn with(
        mut self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
    ) -> SoapResult<Self> {
        self.insert(logical, physical, scalar_kind)?;
        Ok(self)
    }

    /// Inserts one logical-to-physical mapping.
    pub fn insert(
        &mut self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
    ) -> SoapResult<()> {
        let logical = FieldName::new(logical)?;
        let column = PgColumn::new(PgIdentifier::new(physical)?, scalar_kind);
        if self.columns.insert(logical.clone(), column).is_some() {
            return Err(SoapError::validation(format!(
                "duplicate PostgreSQL mapping for logical field `{logical}`"
            )));
        }
        Ok(())
    }

    /// Resolves an allow-listed logical field.
    pub fn resolve(&self, logical: &FieldName) -> SoapResult<&PgColumn> {
        self.columns.get(logical).ok_or_else(|| {
            SoapError::validation(format!("unknown logical field `{logical}` for PostgreSQL"))
        })
    }
}

#[cfg(test)]
mod tests {
    use soaprs_repository::FieldName;

    use super::{PgFieldMap, PgScalarKind};

    #[test]
    fn resolves_only_explicitly_mapped_fields() {
        let mapping = PgFieldMap::new().with("name", "display_name", PgScalarKind::Text);
        let mapping = match mapping {
            Ok(mapping) => mapping,
            Err(error) => panic!("valid mapping failed: {error}"),
        };
        let name = FieldName::new("name");
        let unknown = FieldName::new("unknown");

        assert!(
            name.as_ref()
                .is_ok_and(|field| mapping.resolve(field).is_ok())
        );
        assert!(
            unknown
                .as_ref()
                .is_ok_and(|field| mapping.resolve(field).is_err())
        );
    }
}
