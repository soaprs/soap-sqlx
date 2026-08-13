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
    /// PostgreSQL `uuid` values.
    Uuid,
    /// PostgreSQL `jsonb` values.
    Json,
    /// PostgreSQL `date` values.
    Date,
    /// PostgreSQL `time` values without a time zone.
    Time,
    /// PostgreSQL `timestamp` values without a time zone.
    Timestamp,
    /// PostgreSQL `timestamptz` values.
    TimestampTz,
}

/// Persistence operations allowed for one mapped PostgreSQL field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PgFieldPermissions {
    selectable: bool,
    insertable: bool,
    updatable: bool,
}

impl PgFieldPermissions {
    /// Ordinary persisted field selected, inserted, and updated.
    pub const PERSISTED: Self = Self::new(true, true, true);
    /// Immutable field selected and inserted but never updated.
    pub const IMMUTABLE: Self = Self::new(true, true, false);
    /// Database-generated field selected but omitted from writes.
    pub const GENERATED: Self = Self::new(true, false, false);

    /// Creates an explicit permission set.
    pub const fn new(selectable: bool, insertable: bool, updatable: bool) -> Self {
        Self {
            selectable,
            insertable,
            updatable,
        }
    }

    /// Reports whether the field is included in repository selections.
    pub const fn is_selectable(self) -> bool {
        self.selectable
    }

    /// Reports whether the field participates in inserts.
    pub const fn is_insertable(self) -> bool {
        self.insertable
    }

    /// Reports whether the field participates in replacements.
    pub const fn is_updatable(self) -> bool {
        self.updatable
    }
}

/// Physical PostgreSQL column selected by one logical application field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgColumn {
    identifier: PgIdentifier,
    scalar_kind: PgScalarKind,
    permissions: PgFieldPermissions,
}

impl PgColumn {
    /// Defines a physical column and the scalar family used for bindings.
    pub const fn new(
        identifier: PgIdentifier,
        scalar_kind: PgScalarKind,
        permissions: PgFieldPermissions,
    ) -> Self {
        Self {
            identifier,
            scalar_kind,
            permissions,
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

    /// Returns persistence permissions for this column.
    pub const fn permissions(&self) -> PgFieldPermissions {
        self.permissions
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
        self.insert_with_permissions(
            logical,
            physical,
            scalar_kind,
            PgFieldPermissions::PERSISTED,
        )
    }

    pub(crate) fn insert_with_permissions(
        &mut self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
        permissions: PgFieldPermissions,
    ) -> SoapResult<()> {
        let logical = FieldName::new(logical)?;
        let column = PgColumn::new(PgIdentifier::new(physical)?, scalar_kind, permissions);
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

    pub(crate) fn contains(&self, logical: &FieldName) -> bool {
        self.columns.contains_key(logical)
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
