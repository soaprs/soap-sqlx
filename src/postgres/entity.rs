use soaprs_core::{Entity, SoapError, SoapResult};
use soaprs_repository::{FieldName, ScalarValue};
use sqlx::postgres::PgRow;

use super::{PgFieldMap, PgIdentifier, PgScalarKind};

/// Complete PostgreSQL mapping used by the generic repository.
#[derive(Debug, Clone)]
pub struct PgEntityMapping {
    table: PgIdentifier,
    id_field: FieldName,
    fields: PgFieldMap,
    ordered_fields: Vec<FieldName>,
}

impl PgEntityMapping {
    /// Starts a mapping for one table and logical identity field.
    pub fn new(table: impl Into<String>, id_field: impl Into<String>) -> SoapResult<Self> {
        Ok(Self {
            table: PgIdentifier::new(table)?,
            id_field: FieldName::new(id_field)?,
            fields: PgFieldMap::new(),
            ordered_fields: Vec::new(),
        })
    }

    /// Adds one selected and persisted entity field in statement order.
    pub fn with_field(
        mut self,
        logical: impl Into<String>,
        physical: impl Into<String>,
        scalar_kind: PgScalarKind,
    ) -> SoapResult<Self> {
        let logical = FieldName::new(logical)?;
        self.fields
            .insert(logical.as_str(), physical, scalar_kind)?;
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
        Ok(())
    }

    /// Returns the mapped table.
    pub const fn table(&self) -> &PgIdentifier {
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

    /// Returns mapped fields in deterministic statement order.
    pub fn ordered_fields(&self) -> &[FieldName] {
        &self.ordered_fields
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
    fn value(&self, entity: &E, field: &FieldName) -> SoapResult<ScalarValue>;

    /// Encodes the stable entity identifier.
    fn id_value(&self, id: &E::Id) -> SoapResult<ScalarValue>;
}

#[cfg(test)]
mod tests {
    use soaprs_core::SoapErrorKind;

    use super::PgEntityMapping;
    use crate::postgres::PgScalarKind;

    #[test]
    fn requires_the_identity_field_to_be_mapped() {
        let mapping = PgEntityMapping::new("users", "id")
            .and_then(|mapping| mapping.with_field("name", "display_name", PgScalarKind::Text));
        let error = mapping.and_then(|mapping| mapping.validate());

        assert_eq!(
            error.as_ref().map_err(|error| error.kind()),
            Err(SoapErrorKind::Validation)
        );
    }
}
