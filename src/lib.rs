//! SQLx database adapters for soaprs.
//!
//! The first supported dialect is PostgreSQL. Database representations stay in
//! this crate; domain and application code continue to depend on soaprs ports.

#[cfg(feature = "postgres")]
pub mod postgres;
