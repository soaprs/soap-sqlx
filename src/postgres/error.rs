use soaprs_core::{ErrorTransience, SoapError};
use sqlx::{
    Error,
    error::{DatabaseError, ErrorKind},
};

/// Maps a SQLx failure to the stable soaprs error model while preserving its
/// technical source.
pub fn map_sqlx_error(error: Error, operation: &'static str) -> SoapError {
    let mapped = match &error {
        Error::RowNotFound => SoapError::not_found(format!("{operation} found no row")),
        Error::PoolTimedOut => {
            SoapError::timeout(format!("{operation} timed out acquiring a connection"))
        }
        Error::PoolClosed => {
            SoapError::unavailable(format!("{operation} cannot use a closed connection pool"))
        }
        Error::Io(_) => SoapError::unavailable(format!("{operation} cannot reach PostgreSQL")),
        Error::Tls(_) => {
            SoapError::infrastructure(format!("{operation} cannot establish PostgreSQL TLS"))
                .with_transience(ErrorTransience::Permanent)
        }
        Error::Database(database) => map_database_error(database.as_ref(), operation),
        Error::Configuration(_)
        | Error::Protocol(_)
        | Error::ColumnDecode { .. }
        | Error::ColumnIndexOutOfBounds { .. }
        | Error::ColumnNotFound(_)
        | Error::Decode(_)
        | Error::Encode(_)
        | Error::InvalidArgument(_)
        | Error::Migrate(_)
        | Error::TypeNotFound { .. } => {
            SoapError::infrastructure(format!("{operation} has an invalid PostgreSQL mapping"))
                .with_transience(ErrorTransience::Permanent)
        }
        _ => SoapError::infrastructure(format!("{operation} failed in SQLx")),
    };

    mapped.with_source(error)
}

fn map_database_error(database: &dyn DatabaseError, operation: &'static str) -> SoapError {
    let sqlstate = database.code();
    if matches!(sqlstate.as_deref(), Some("40001" | "40P01")) {
        return SoapError::infrastructure(format!(
            "{operation} encountered a concurrent PostgreSQL transaction"
        ))
        .with_transience(ErrorTransience::Transient);
    }
    if sqlstate
        .as_deref()
        .is_some_and(|code| code.starts_with("08"))
    {
        return SoapError::unavailable(format!("{operation} lost its PostgreSQL connection"));
    }

    match database.kind() {
        ErrorKind::UniqueViolation | ErrorKind::ExclusionViolation => {
            SoapError::conflict(format!("{operation} conflicts with an existing row"))
        }
        ErrorKind::ForeignKeyViolation => {
            SoapError::conflict(format!("{operation} violates a relationship constraint"))
        }
        ErrorKind::NotNullViolation | ErrorKind::CheckViolation => {
            SoapError::infrastructure(format!("{operation} violates the persisted schema"))
                .with_transience(ErrorTransience::Permanent)
        }
        _ => SoapError::infrastructure(format!("{operation} failed in PostgreSQL"))
            .with_transience(ErrorTransience::Permanent),
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, error::Error as _, fmt, io};

    use soaprs_core::{ErrorTransience, SoapErrorKind};
    use sqlx::{
        Error,
        error::{DatabaseError, ErrorKind},
    };

    use super::map_sqlx_error;

    #[test]
    fn classifies_pool_timeout_as_transient_and_preserves_source() {
        let error = map_sqlx_error(Error::PoolTimedOut, "find users");

        assert_eq!(error.kind(), SoapErrorKind::Timeout);
        assert_eq!(error.transience(), ErrorTransience::Transient);
        assert!(error.source().is_some());
    }

    #[test]
    fn classifies_io_failure_as_unavailable() {
        let error = map_sqlx_error(
            Error::Io(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "driver detail",
            )),
            "connect",
        );

        assert_eq!(error.kind(), SoapErrorKind::Unavailable);
        assert!(error.is_reportable());
        assert!(!error.to_string().contains("driver detail"));
    }

    #[test]
    fn classifies_mapping_failure_as_permanent_infrastructure() {
        let error = map_sqlx_error(Error::ColumnNotFound("name".into()), "decode user");

        assert_eq!(error.kind(), SoapErrorKind::Infrastructure);
        assert_eq!(error.transience(), ErrorTransience::Permanent);
    }

    #[test]
    fn maps_unique_constraint_to_conflict_and_preserves_database_source() {
        let error = Error::Database(Box::new(MockDatabaseError {
            kind: MockErrorKind::Unique,
            code: "23505",
        }));
        let error = map_sqlx_error(error, "insert user");

        assert_eq!(error.kind(), SoapErrorKind::Conflict);
        assert!(error.source().is_some());
    }

    #[test]
    fn marks_serialization_and_deadlock_failures_as_transient() {
        for code in ["40001", "40P01"] {
            let error = Error::Database(Box::new(MockDatabaseError {
                kind: MockErrorKind::Other,
                code,
            }));
            let error = map_sqlx_error(error, "replace user");

            assert_eq!(error.kind(), SoapErrorKind::Infrastructure);
            assert_eq!(error.transience(), ErrorTransience::Transient);
        }
    }

    #[test]
    fn marks_unrecognized_database_failures_as_permanent() {
        let error = Error::Database(Box::new(MockDatabaseError {
            kind: MockErrorKind::Other,
            code: "42703",
        }));
        let error = map_sqlx_error(error, "find users");

        assert_eq!(error.kind(), SoapErrorKind::Infrastructure);
        assert_eq!(error.transience(), ErrorTransience::Permanent);
        assert!(error.source().is_some());
    }

    #[derive(Debug)]
    enum MockErrorKind {
        Unique,
        Other,
    }

    #[derive(Debug)]
    struct MockDatabaseError {
        kind: MockErrorKind,
        code: &'static str,
    }

    impl fmt::Display for MockDatabaseError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("mock PostgreSQL error")
        }
    }

    impl std::error::Error for MockDatabaseError {}

    impl DatabaseError for MockDatabaseError {
        fn message(&self) -> &str {
            "mock PostgreSQL error"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> ErrorKind {
            match self.kind {
                MockErrorKind::Unique => ErrorKind::UniqueViolation,
                MockErrorKind::Other => ErrorKind::Other,
            }
        }
    }
}
