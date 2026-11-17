//! Official Rust client for the [BillionVerify](https://billionverify.com) email verification API.
//!
//! ```no_run
//! # async fn run() -> billionverify::Result<()> {
//! let client = billionverify::Client::new("your_api_key")?;
//! let result = client.verify("user@example.com", Default::default()).await?;
//! println!("{} -> {}", result.email, result.status);
//! # Ok(()) }
//! ```

mod client;
mod error;
mod types;
mod webhook;

pub use client::{Client, ClientBuilder, DEFAULT_BASE_URL};
pub use error::{Error, ErrorInfo, Result};
pub use types::*;
pub use webhook::verify_webhook_signature;

/// Version of this crate, sent in the `User-Agent` header.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
