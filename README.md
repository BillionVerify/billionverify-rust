# billionverify

Official Rust client for the [BillionVerify](https://billionverify.com) email verification API.

It covers every operation of the API: single and batch verification, the free disposable check, file verification jobs, credits, webhooks, domain search and batch domain search.

## Installation

```toml
[dependencies]
billionverify = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Requires Rust 1.75 or newer. TLS is rustls.

## Quick start

```rust
use billionverify::Client;

#[tokio::main]
async fn main() -> billionverify::Result<()> {
    let client = Client::new(std::env::var("BILLIONVERIFY_API_KEY").unwrap())?;

    let result = client.verify("user@example.com", Default::default()).await?;
    println!("{} -> {} (score {})", result.email, result.status, result.score);
    Ok(())
}
```

Configure the client with the builder:

```rust
use std::time::Duration;

let client = billionverify::Client::builder()
    .api_key("your_api_key")
    .timeout(Duration::from_secs(60)) // default 30 s
    .retries(5)                       // total attempts per request, default 3
    .build()?;
```

`base_url` defaults to `https://api.billionverify.com`; a trailing `/v1` is ignored. The key is sent in the `BV-API-KEY` header.

## Verification

```rust
use billionverify::VerifyOptions;

// One address. Options you leave unset are not sent, so the server default applies.
let r = client
    .verify("user@example.com", VerifyOptions { force_refresh: Some(true), ..Default::default() })
    .await?;

// Up to 50 addresses in one request.
let batch = client
    .verify_batch(&["a@example.com".into(), "b@example.com".into()], Default::default())
    .await?;
println!("{:?}", batch.deliverability_buckets);

// Free, no credits: is the domain disposable?
let d = client.check_disposable("user@mailinator.com").await?;
let d = client.check_disposable_via_query("user@mailinator.com").await?;
```

More than 50 addresses in `verify_batch` is rejected locally with `Error::Validation`; use a file job for larger lists.

## File jobs

```rust
use std::time::Duration;
use billionverify::{ResultFilters, UploadOptions};

let upload = client.upload_file_path("emails.csv", UploadOptions::default()).await?;
// or: client.upload_file(bytes, "emails.csv", UploadOptions::default()).await?

// Poll until finished (a paused job also returns, it never advances by itself).
let status = client
    .wait_for_file_job(&upload.task_id, Duration::from_secs(5), Duration::from_secs(600))
    .await?;

// Or let the server hold the request for up to 300 seconds.
let status = client.get_file_job_status(&upload.task_id, Some(60)).await?;

// Result CSV, optionally filtered (filters combine with OR).
let csv = client
    .download_file_results(&upload.task_id, ResultFilters { valid: true, role: true, ..Default::default() })
    .await?;

// Resume a paused job.
client.resume_job(&upload.task_id).await?;
```

The results endpoint redirects to pre-signed storage. The client follows the redirect itself and does not send your API key to the storage host.

## Credits and webhooks

```rust
let credits = client.get_credits().await?;
println!("{} credits left", credits.credits_balance);

let hook = client
    .create_webhook("https://your-app.com/webhooks/billionverify", &["file.completed".into(), "file.failed".into()])
    .await?;
let secret = hook.secret; // returned only here

let hooks = client.list_webhooks().await?;
client.delete_webhook(&hook.id).await?;
```

### Verifying webhook deliveries

The signature covers `{X-Webhook-Timestamp}.{raw_body}` (HMAC-SHA256, hex, header format `sha256=<hex>`). Pass the raw body exactly as received:

```rust
use billionverify::verify_webhook_signature;

let ok = verify_webhook_signature(
    raw_body,            // &[u8]
    signature_header,    // X-Webhook-Signature
    timestamp_header,    // X-Webhook-Timestamp
    &secret,
    300,                 // reject timestamps more than 5 minutes off; 0 disables the check
);
```

## Domain search

```rust
use std::time::Duration;
use billionverify::{DomainSearchBatchOptions, DomainSearchOptions, ListBatchesOptions};

// Emails published on a company's site, each verified. max_emails (1-200) caps the bill.
let job = client.create_domain_search("example.com", 20, DomainSearchOptions::default()).await?;
let search = client
    .wait_for_domain_search(&job.search_id, Duration::from_secs(3), Duration::from_secs(300))
    .await?;
println!("{:?}", search.result);

// A CSV of domains.
let batch = client
    .create_domain_search_batch(std::fs::read("domains.csv").unwrap(), "domains.csv", DomainSearchBatchOptions::default())
    .await?;
let progress = client.get_domain_search_batch(&batch.batch_id).await?;
let page = client.list_domain_search_batches(ListBatchesOptions::default()).await?;
let csv = client.download_domain_search_batch_results(&batch.batch_id).await?; // 409 until finished
```

## Health check

```rust
let health = client.health_check().await?; // no authentication
```

## Errors

Every method returns `billionverify::Result<T>`. `Error` has one variant per case, all carrying `message()`, `code()`, `status_code()` and `details()`:

| Variant | When |
| --- | --- |
| `Authentication` | 401, or an empty API key |
| `Validation` | 400, or a local check failed (more than 50 emails, `max_emails` out of range, timeout over 300) |
| `InsufficientCredits` | 402 |
| `NotFound` | 404 |
| `RateLimit { info, retry_after }` | 429 after retries ran out |
| `Timeout` | the request or a wait timed out (never retried) |
| `Network` | no response (never retried) |
| `Api` | anything else, 409 and 5xx included, and unreadable responses (`INVALID_RESPONSE`) |

Requests are retried on 429, 500, 502 and 503: after the `Retry-After` header for a 429 that has one, otherwise after 2, 4, 8 ... seconds.

## Releasing

1. Update `version` in `Cargo.toml` and add an entry to `CHANGELOG.md`.
2. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`.
3. Commit, tag `vX.Y.Z` and push the tag. The `release` workflow runs the checks, then publishes with the `CARGO_REGISTRY_TOKEN` repository secret.
4. To publish by hand instead: `cargo publish --dry-run`, then `cargo publish`.

## License

MIT
