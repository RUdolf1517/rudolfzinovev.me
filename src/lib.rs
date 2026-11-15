//! The site is one binary crate whose HTTP layer is axum/tokio.
//!
//! `main.rs` owns the runtime and the CLI; every module here is a plain library
//! module. Nothing below the transport links a crate: SQLite and OpenSSL are
//! reached through FFI, and passwords, Markdown, templates, i18n, the GitHub
//! client and its JSON parser are all hand-written.

pub mod crypto;
pub mod db;
pub mod github;
pub mod i18n;
pub mod json;
pub mod markdown;
pub mod models;
pub mod sqlite;
pub mod templates;

// The HTTP layer.
pub mod handlers;
pub mod state;
pub mod util;

pub use db::Store;
pub use models::{Lang, Profile};
