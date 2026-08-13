use soaprs_core::{Entity, SoapError, SoapResult};
use soaprs_repository::FieldName;
use sqlx::postgres::PgRow;

use super::{PgFieldMap, PgFieldPermissions, PgIdentifier, PgScalarKind, PgValue};

/// Validated optionally schema-qualified PostgreSQL table name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PgTable {
    schema: Option<PgIdentifier>,
    name: PgIdentifier,
}

impl PgTable {
    /// Creates an unqualified table name resolved through PostgreSQL search path.
    pub fn new(name: impl Into<String>) -> SoapResult<Self> {
        Ok(Self {
            schema: None,
            name: PgIdentifier::new(name)?,
        })
    }

    /// Creates an explicitly schema-qualified table name.
    pub fn in_schema(schema: impl Into<String>, name: impl Into<String>) -> SoapResult<Self> {
        Ok(Self {
            schema: Some(PgIdentifier::new(schema)?),
            name: PgIdentifier::new(name)?,
        })
    }

    /// Returns the optional schema name.
    pub const fn schema(&self) -> Option<&PgIdentifier> {
        self.schema.as_ref()
    }

    /// Returns the table name without its schema.
    pub const fn name(&self) -> &PgIdentifier {
        &self.name
    }

    pub(crate) fn quoted(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", schema.quoted(), self.name.quoted()),
            None => self.name.quoted(),
        }
    }
}

/// Complete PostgreSQL mapping used by the generic repository.
#[derive(Debug, Clone)]
pub struct PgEntityMapping {
    table: PgTable,
    id_field: FieldName,
    fields: PgFieldMap,
    ordered_fields: Vec<FieldName>,
}

impl PgEntityMapping {
    /// Starts a mapping for one unqualified table and logical identity field.
    pub fn new(table: impl Into<String>, id_field: impl Into<String>) -> SoapResult<Self> {
        Ok(Self {
            table: PgTable::new(table)?,
            id_field: FieldName::new(id_field)?,
            fields: PgFieldMap::new(),
            ordered_fields: Vec::new(),
        })
    }

    /// Starts a mapping for a table in an explicit PostgreSQL schema.
    pub fn in_schema(
        schema: impl Into<String>,
        table: impl Into<String>,
        id_field: impl Into<String>,
    ) -> SoapResult<Self> {
        Ok(Self {
            table: PgTable::in_schema(schema, table)?,
            id_field: FieldName::new(id_field)?,
            fields: PgFieldMap::new(),
            ordered_fields: Vec::new(),
        })
    }

    /// Adds an ordinary selected, inserted, and updated entity field.
    pub fn with_field(
        self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
    ) -> SoapResult<Self> {
        self.with_field_permissions(
            logical,
            physical,
            scalar_kind,
            PgFieldPermissions::PERSISTED,
        )
    }

    /// Adds a selected and inserted field that replacements never update.
    pub fn with_immutable_field(
        self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
    ) -> SoapResult<Self> {
        self.with_field_permissions(
            logical,
            physical,
            scalar_kind,
            PgFieldPermissions::IMMUTABLE,
        )
    }

    /// Adds a database-generated field omitted from inserts and replacements.
    pub fn with_generated_field(
        self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
    ) -> SoapResult<Self> {
        self.with_field_permissions(
            logical,
            physical,
            scalar_kind,
            PgFieldPermissions::GENERATED,
        )
    }

    /// Adds a field with explicit selection and persistence permissions.
    pub fn with_field_permissions(
        mut self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
        permissions: PgFieldPermissions,
    ) -> SoapResult<Self> {
        if !permissions.is_selectable()
            && !permissions.is_insertable()
            && !permissions.is_updatable()
        {
            return Err(SoapError::validation(
                "PostgreSQL mapped field must participate in at least one operation",
            ));
        }
        let logical = FieldName::new(logical)?;
        self.fields.insert_with_permissions(
            logical.as_str(),
            physical,
            scalar_kind,
            permissions,
        )?;
        self.ordered_fields.push(logical);
        Ok(self)
    }

    /// Validates that the mapping is complete enough for repository use.
    pub fn validate(&self) -> SoapResult<()> {
        if self.ordered_fields.is_empty() {
            return Err(SoapError::validation(
                "PostgreSQL entity mapping must contain at least one field",
            ));
        }
        if !self.fields.contains(&self.id_field) {
            return Err(SoapError::validation(format!(
                "PostgreSQL identity field `{}` is not mapped",
                self.id_field
            )));
        }
        let id = self.fields.resolve(&self.id_field)?;
        if !id.permissions().is_selectable() {
            return Err(SoapError::validation(
                "PostgreSQL identity field must be selectable",
            ));
        }
        if self.selectable_fields().next().is_none() {
            return Err(SoapError::validation(
                "PostgreSQL entity mapping must select at least one field",
            ));
        }
        Ok(())
    }

    /// Returns the mapped table.
    pub const fn table(&self) -> &PgTable {
        &self.table
    }

    /// Returns the logical identity field.
    pub const fn id_field(&self) -> &FieldName {
        &self.id_field
    }

    /// Returns the field allow-list used by the query compiler.
    pub const fn fields(&self) -> &PgFieldMap {
        &self.fields
    }

    /// Returns all mapped fields in deterministic statement order.
    pub fn ordered_fields(&self) -> &[FieldName] {
        &self.ordered_fields
    }

    pub(crate) fn selectable_fields(&self) -> impl Iterator<Item = &FieldName> {
        self.fields_matching(|permissions| permissions.is_selectable())
    }

    pub(crate) fn insertable_fields(&self) -> impl Iterator<Item = &FieldName> {
        self.fields_matching(|permissions| permissions.is_insertable())
    }

    pub(crate) fn updatable_fields(&self) -> impl Iterator<Item = &FieldName> {
        self.fields_matching(|permissions| permissions.is_updatable())
    }

    fn fields_matching(
        &self,
        predicate: impl Fn(PgFieldPermissions) -> bool + Copy,
    ) -> impl Iterator<Item = &FieldName> {
        self.ordered_fields.iter().filter(move |field| {
            self.fields
                .resolve(field)
                .is_ok_and(|column| predicate(column.permissions()))
        })
    }
}

/// Adapter-owned conversion between a domain entity and PostgreSQL rows.
///
/// Implementations belong in application infrastructure. Domain entities do
/// not implement SQLx traits and remain independent from physical columns.
pub trait PgEntityCodec<E>: Send + Sync
where
    E: Entity,
{
    /// Returns the table and field mapping for this codec.
    fn mapping(&self) -> &PgEntityMapping;

    /// Decodes one complete entity from a selected PostgreSQL row.
    fn decode(&self, row: &PgRow) -> SoapResult<E>;

    /// Encodes one non-identity logical field from an entity.
    fn value(&self, entity: &E, field: &FieldName) -> SoapResult<PgValue>;

    /// Encodes the stable entity identifier.
    fn id_value(&self, id: &E::Id) -> SoapResult<PgValue>;
}

#[cfg(test)]
mod tests {
    use soaprs_core::SoapErrorKind;

    use super::PgEntityMapping;
    use crate::postgres::{PgFieldPermissions, PgScalarKind};

    #[test]
    fn requires_the_identity_field_to_be_mapped_and_selectable() {
        let missing = PgEntityMapping::new("users", "id")
            .and_then(|mapping| mapping.with_field("name", "display_name", PgScalarKind::Text))
            .and_then(|mapping| mapping.validate());
        let hidden = PgEntityMapping::new("users", "id")
            .and_then(|mapping| {
                mapping.with_field_permissions(
                    "id",
                    "id",
                    PgScalarKind::Numeric,
                    PgFieldPermissions::new(false, true, false),
                )
            })
            .and_then(|mapping| mapping.validate());

        assert_eq!(
            missing.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
        assert_eq!(
            hidden.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
    }

    #[test]
    fn supports_qualified_tables_and_field_roles() {
        let mapping = PgEntityMapping::in_schema("billing", "users", "id")
            .and_then(|mapping| mapping.with_immutable_field("id", "id", PgScalarKind::Uuid))
            .and_then(|mapping| {
                mapping.with_generated_field("created_at", "created_at", PgScalarKind::TimestampTz)
            });
        let mapping = match mapping {
            Ok(mapping) => mapping,
            Err(error) => panic!("valid qualified mapping failed: {error}"),
        };

        assert_eq!(mapping.table().quoted(), r#""billing"."users""#);
        assert_eq!(mapping.insertable_fields().count(), 1);
        assert_eq!(mapping.updatable_fields().count(), 0);
        assert_eq!(mapping.selectable_fields().count(), 2);
    }
}
