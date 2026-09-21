//! Credential-bearing data stays in this crate.  Public DTOs contain only quota values.
pub mod auth;
pub mod providers;
pub mod quota;
pub mod activity;
pub mod settings;
pub mod update;
#[cfg(feature = "desktop")]
pub mod window;
#[cfg(feature = "desktop")]
pub mod app;
