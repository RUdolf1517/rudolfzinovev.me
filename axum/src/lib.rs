//! Shared core: everything in the parent crate except its HTTP server.
//!
//! The zero-crate modules (SQLite FFI, OpenSSL FFI, password hashing, Markdown,
//! templates, i18n, models) are reused verbatim through `#[path]`. Only the
//! transport (axum/tokio) and the request/response adaptation are new, plus
//! `util` which replaces the two helpers that used to live in `http.rs`.

// ---- reused verbatim from the zero-crate implementation -------------------
#[path = "../../src/crypto.rs"]
pub mod crypto;
#[path = "../../src/i18n.rs"]
pub mod i18n;
#[path = "../../src/markdown.rs"]
pub mod markdown;
#[path = "../../src/models.rs"]
pub mod models;
#[path = "../../src/sqlite.rs"]
pub mod sqlite;
#[path = "../../src/db.rs"]
pub mod db;
#[path = "../../src/templates.rs"]
pub mod templates;

// ---- new in this crate ----------------------------------------------------
pub mod handlers;
pub mod state;
pub mod util;

pub use db::Store;
pub use models::{Lang, Profile};
