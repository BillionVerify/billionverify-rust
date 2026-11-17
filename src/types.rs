//! Request options and response types.
//!
//! Response structs ignore fields they do not know, and optional fields default to `None`, so a
//! newer API never breaks an older client.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Options for [`Client::verify`](crate::Client::verify) and
/// [`Client::verify_batch`](crate::Client::verify_batch). Unset options are not sent, so the
/// server default applies.
#[derive(Debug, Clone, Default)]
pub struct VerifyOptions {
    /// Override the SMTP check for this request.
    pub check_smtp: Option<bool>,
    /// Bypass every cache and run a live verification (always costs a credit).
    pub force_refresh: Option<bool>,
    /// Also fetch the MX IP reputation when a cached result is returned.
    pub include_domain_reputation: Option<bool>,
}

/// Options for [`Client::upload_file`](crate::Client::upload_file).
#[derive(Debug, Clone, Default)]
pub struct UploadOptions {
    /// Override the SMTP check for the whole file.
    pub check_smtp: Option<bool>,
    /// Column that holds the addresses (auto-detected when unset).
    pub email_column: Option<String>,
    /// Keep the original columns in the result file (server default: true).
    pub preserve_original: Option<bool>,
}

/// Status filters for [`Client::download_file_results`](crate::Client::download_file_results).
/// Several filters combine with OR; with none set the full result file is returned.
#[derive(Debug, Clone, Default)]
pub struct ResultFilters {
    /// Include valid addresses.
    pub valid: bool,
    /// Include invalid addresses.
    pub invalid: bool,
    /// Include catch-all addresses.
    pub catchall: bool,
    /// Include role addresses.
    pub role: bool,
    /// Include unknown addresses.
    pub unknown: bool,
    /// Include disposable addresses.
    pub disposable: bool,
    /// Include risky addresses.
    pub risky: bool,
}

/// Options for [`Client::create_domain_search`](crate::Client::create_domain_search).
#[derive(Debug, Clone, Default)]
pub struct DomainSearchOptions {
    /// Keep `info@` / `sales@` style addresses (server default: true).
    pub include_role_based: Option<bool>,
    /// Sitemap-wide crawl, truncated at this many pages.
    pub max_pages: Option<u32>,
}

/// Options for [`Client::create_domain_search_batch`](crate::Client::create_domain_search_batch).
#[derive(Debug, Clone, Default)]
pub struct DomainSearchBatchOptions {
    /// Cap on emails per domain, 1-200 (server default: 200).
    pub max_emails: Option<u32>,
    /// Keep role addresses (server default: true).
    pub include_role_based: Option<bool>,
    /// Sitemap-wide crawl per domain, truncated at this many pages.
    pub max_pages: Option<u32>,
    /// Column mapping, for example `{"domain": "Company Website"}`.
    pub column_map: Option<Value>,
}

/// Options for [`Client::list_domain_search_batches`](crate::Client::list_domain_search_batches).
#[derive(Debug, Clone, Default)]
pub struct ListBatchesOptions {
    /// Page size (server default 20, maximum 100).
    pub limit: Option<u32>,
    /// Only batches in this status.
    pub status: Option<String>,
    /// Cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
}

/// MX IP reputation attached to a verification result. Display-only.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainReputation {
    /// MX IP address.
    pub mx_ip: Option<String>,
    /// Whether the IP is on any blacklist.
    pub is_listed: bool,
    /// Blacklists that list the IP.
    pub blacklists: Vec<String>,
    /// Graded, non-listing answers.
    pub advisory: Vec<String>,
    /// Whether the check ran.
    pub checked: bool,
    /// MX IP is inside a Spamhaus DROP netblock.
    pub drop_listed: bool,
    /// Matching DROP netblock.
    pub drop_cidr: Option<String>,
    /// Spamhaus SBL reference.
    pub drop_sblid: Option<String>,
    /// Required attribution text for the DROP data.
    pub drop_attribution: Option<String>,
}

/// The result of verifying one address.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct VerificationResult {
    /// The verified address.
    pub email: String,
    /// `valid`, `invalid`, `unknown`, `risky`, `disposable`, `catchall` or `role`.
    pub status: String,
    /// Quality score, 0 to 1.
    pub score: f64,
    /// The verification channel confirmed the address is deliverable.
    pub is_deliverable: bool,
    /// Disposable / temporary address.
    pub is_disposable: bool,
    /// The domain accepts all addresses.
    pub is_catchall: bool,
    /// Role address (`info@`, `support@`, ...).
    pub is_role: bool,
    /// Free mailbox provider.
    pub is_free: bool,
    /// Address domain.
    pub domain: String,
    /// Domain age in years, when known.
    pub domain_age: Option<i64>,
    /// MX records of the domain.
    pub mx_records: Option<Vec<String>>,
    /// Primary MX IP.
    pub mx_ip: Option<String>,
    /// Primary MX autonomous system number.
    pub mx_asn: Option<i64>,
    /// Primary MX autonomous system organization.
    pub mx_as_org: Option<String>,
    /// MX IP reputation.
    pub domain_reputation: Option<DomainReputation>,
    /// Whether the result carries SMTP data.
    pub check_smtp: bool,
    /// `not_checked`, `deliverable`, `undeliverable` or `inconclusive`.
    pub smtp_result: Option<String>,
    /// How the catch-all probe behaved.
    pub catch_all_evidence: Option<String>,
    /// Detailed reason.
    pub reason: Option<String>,
    /// Deprecated; use `domain_suggestion`.
    pub suggestion: Option<String>,
    /// Suggested correction for a typo.
    pub domain_suggestion: Option<String>,
    /// Canonical address when the submitted one is an alias.
    pub alias_of: Option<String>,
    /// External risk signals behind a `risky` status.
    pub risk_reasons: Option<Vec<String>>,
    /// Redacted SMTP response (allow-listed accounts only).
    pub smtp_response: Option<String>,
    /// Response time in milliseconds.
    pub response_time: i64,
    /// Credits consumed.
    pub credits_used: i64,
    /// Which verification path produced the result.
    pub verification_mode: Option<String>,
    /// Whether `force_refresh` could change the result.
    pub retryable: bool,
    /// The local part needs SMTPUTF8.
    pub smtputf8: bool,
    /// Retry hint from a transient SMTP response, in seconds.
    pub retry_after_seconds: Option<i64>,
}

/// Pre-aggregated deliverability buckets.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DeliverabilityBuckets {
    /// valid + role.
    pub strict_deliverable: i64,
    /// strict_deliverable + catchall.
    pub usable_with_catchall: i64,
    /// invalid + disposable + risky.
    pub not_deliverable: i64,
    /// All unknown rows.
    pub unknown: i64,
    /// Unknown rows from providers that refuse SMTP probing.
    pub anti_verified: i64,
    /// Unknown rows a retry may improve.
    pub transient_failure: i64,
}

/// The result of a batch verification.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct BatchVerificationResult {
    /// One result per address.
    pub results: Vec<VerificationResult>,
    /// Addresses submitted.
    pub total_emails: i64,
    /// Valid addresses.
    pub valid_emails: i64,
    /// Invalid addresses.
    pub invalid_emails: i64,
    /// Credits consumed.
    pub credits_used: i64,
    /// Processing time in milliseconds.
    pub process_time: i64,
    /// Addresses a retry with `force_refresh` may improve.
    pub retryable_emails: Option<Vec<String>>,
    /// Addresses deferred to async processing.
    pub deferred_emails: Option<Vec<String>>,
    /// Hint for when to retry, in seconds.
    pub retry_after_seconds: Option<i64>,
    /// Aggregated buckets.
    pub deliverability_buckets: Option<DeliverabilityBuckets>,
}

/// The free disposable-domain check.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DisposableCheck {
    /// The address checked.
    pub email: String,
    /// Its domain.
    pub domain: String,
    /// Whether the domain is disposable.
    pub is_disposable: bool,
    /// When the check ran (RFC 3339).
    pub checked_at: Option<String>,
}

/// Response of a file upload.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct FileUpload {
    /// Job id; poll it with `get_file_job_status`.
    pub task_id: String,
    /// File name.
    pub file_name: String,
    /// Size in bytes.
    pub file_size: i64,
    /// `pending`, or `preparing` for large uploads.
    pub status: String,
    /// Status message.
    pub message: String,
    /// URL to poll.
    pub status_url: String,
    /// Creation time (RFC 3339).
    pub created_at: Option<String>,
    /// Estimated address count.
    pub estimated_count: i64,
    /// Distinct addresses after dedup.
    pub unique_emails: Option<i64>,
    /// Rows excluding the header.
    pub total_rows: Option<i64>,
    /// Detected address column.
    pub email_column: Option<String>,
    /// Other columns that also hold addresses.
    pub email_column_candidates: Option<Vec<String>>,
}

/// Capacity wait of a file job.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CapacityWait {
    /// Addresses waiting for capacity.
    pub waiting_emails: i64,
    /// Next attempt time (not a completion estimate).
    pub next_eligible_at: Option<String>,
    /// Waiting since.
    pub waiting_since: Option<String>,
}

/// Runtime telemetry of a file job.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RuntimeHints {
    /// A worker is processing the job right now.
    pub has_live_execution: Option<bool>,
    /// Batches in flight.
    pub inflight_batches: Option<i64>,
    /// Last live progress report.
    pub last_execution_fact_at: Option<String>,
}

/// Status of a file verification job.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct FileJobStatus {
    /// Job id.
    pub task_id: String,
    /// `preparing`, `pending`, `running`, `processing`, `merging`, `paused`, `completed`,
    /// `completed_with_warning`, `failed` or `cancelled`.
    pub status: String,
    /// `verifying`, `retry_tail` or `generating_report` while processing.
    pub stage: Option<String>,
    /// Addresses waiting on retry windows (stage `retry_tail`).
    pub pending_retry_emails: Option<i64>,
    /// Progress, 0-100.
    pub progress: i64,
    /// Progress by addresses.
    pub email_progress: Option<i64>,
    /// Progress by chunks.
    pub chunk_progress: Option<i64>,
    /// `email` or `chunk`.
    pub progress_source: Option<String>,
    /// Total addresses.
    pub total_emails: i64,
    /// Addresses processed.
    pub processed_emails: i64,
    /// Valid addresses.
    pub valid_emails: i64,
    /// Invalid addresses.
    pub invalid_emails: i64,
    /// Role addresses.
    pub role_emails: i64,
    /// Catch-all addresses.
    pub catchall_emails: i64,
    /// Unknown addresses.
    pub unknown_emails: i64,
    /// Risky addresses.
    pub risky_emails: i64,
    /// Disposable addresses.
    pub disposable_emails: i64,
    /// Disposable addresses that were charged.
    pub disposable_charged_emails: i64,
    /// Credits consumed.
    pub credits_used: i64,
    /// Rows served from the user cache without charge.
    pub cache_hit_not_charged: Option<i64>,
    /// Start time.
    pub started_at: Option<String>,
    /// Completion time.
    pub completed_at: Option<String>,
    /// Failure message.
    pub error_message: Option<String>,
    /// Why the job is paused.
    pub pause_reason: Option<String>,
    /// Whether the job can be paused.
    pub can_pause: Option<bool>,
    /// Whether the job can be resumed.
    pub can_resume: Option<bool>,
    /// Whether the job can be restarted.
    pub can_restart: Option<bool>,
    /// Restart attempts so far.
    pub restart_attempts: Option<i64>,
    /// Current attempt.
    pub current_attempt: Option<i64>,
    /// API URL of the results CSV.
    pub download_url: Option<String>,
    /// Pre-signed storage URL of the results CSV.
    pub direct_download_url: Option<String>,
    /// Expiry of `direct_download_url`.
    pub direct_download_expires_at: Option<String>,
    /// Whether the result download is available.
    pub result_download_available: bool,
    /// Download deadline, currently always null.
    pub result_download_expires_at: Option<String>,
    /// Why the download is unavailable.
    pub result_download_unavailable_reason: Option<String>,
    /// Chunks in total.
    pub total_chunks: Option<i64>,
    /// Chunks completed.
    pub completed_chunks: Option<i64>,
    /// Chunks failed.
    pub failed_chunks: Option<i64>,
    /// Capacity wait, when remaining addresses await capacity.
    pub capacity_wait: Option<CapacityWait>,
    /// Runtime telemetry.
    pub runtime_hints: Option<RuntimeHints>,
    /// Aggregated buckets.
    pub deliverability_buckets: Option<DeliverabilityBuckets>,
    /// Distinct addresses after dedup (used for billing).
    pub unique_emails: Option<i64>,
    /// Input rows excluding the header.
    pub total_rows: Option<i64>,
}

impl FileJobStatus {
    /// True when the job reached a state that never changes by itself.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "completed_with_warning" | "failed" | "cancelled"
        )
    }
}

/// Credit balance of the account.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Credits {
    /// Account id.
    pub account_id: String,
    /// API key id.
    pub api_key_id: String,
    /// API key name.
    pub api_key_name: String,
    /// Current balance.
    pub credits_balance: i64,
    /// Credits consumed.
    pub credits_consumed: i64,
    /// Credits added.
    pub credits_added: i64,
    /// Last update (RFC 3339).
    pub last_updated: Option<String>,
}

/// A webhook endpoint.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Webhook {
    /// Webhook id.
    pub id: String,
    /// HTTPS URL.
    pub url: String,
    /// Subscribed events.
    pub events: Vec<String>,
    /// Signing secret; returned only when the webhook is created.
    pub secret: Option<String>,
    /// Whether the webhook is active.
    pub is_active: bool,
    /// `success` or `failed`.
    pub last_delivery_status: Option<String>,
    /// Time of the last delivery.
    pub last_delivery_at: Option<String>,
    /// Error of the last failed delivery.
    pub last_error: Option<String>,
    /// Creation time.
    pub created_at: Option<String>,
    /// Update time.
    pub updated_at: Option<String>,
}

/// A queued domain search.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainSearchJob {
    /// Search id.
    pub search_id: String,
    /// `queued` at creation.
    pub status: String,
    /// Domain being searched.
    pub domain: String,
    /// Credits held.
    pub credits_reserved: i64,
    /// URL to poll.
    pub status_url: String,
}

/// Status and result of a domain search.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainSearch {
    /// Search id.
    pub search_id: String,
    /// Status; terminal states are `completed`, `completed_with_warning` and `failed`.
    pub status: String,
    /// Domain searched.
    pub domain: String,
    /// Credits charged.
    pub credits_charged: i64,
    /// Result with `emails`, `crawl_report` and `warnings`, once finished.
    pub result: Option<Value>,
}

impl DomainSearch {
    /// True when the search finished.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "completed_with_warning" | "failed"
        )
    }
}

/// A queued batch domain search.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainSearchBatchJob {
    /// Batch id.
    pub batch_id: String,
    /// `queued` at creation.
    pub status: String,
    /// Rows in the CSV.
    pub total_rows: i64,
    /// Distinct domains.
    pub unique_domains: i64,
    /// Cap on emails per domain.
    pub max_emails: i64,
    /// Whether role addresses are kept.
    pub include_role_based: bool,
    /// Page cap per domain.
    pub max_pages: Option<i64>,
    /// URL to poll.
    pub status_url: String,
    /// URL of the results CSV.
    pub results_url: String,
}

/// Progress of a batch domain search.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainSearchBatchProgress {
    /// Rows in total.
    pub total_rows: i64,
    /// Rows processed.
    pub processed: i64,
    /// Rows with at least one email.
    pub found: i64,
    /// Emails returned.
    pub emails_returned: i64,
    /// Distinct domains.
    pub unique_domains: i64,
}

/// Status of a batch domain search.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainSearchBatch {
    /// Batch id.
    pub batch_id: String,
    /// `queued`, `running`, `completed`, `completed_with_warning`, `failed` or `cancelled`.
    pub status: String,
    /// Cap on emails per domain.
    pub max_emails: i64,
    /// Page cap per domain.
    pub max_pages: Option<i64>,
    /// Progress counters.
    pub progress: DomainSearchBatchProgress,
    /// Credits charged so far.
    pub credits_charged: i64,
}

/// One page of batch domain searches.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DomainSearchBatchPage {
    /// Batches on this page (free-form objects).
    pub items: Vec<Value>,
    /// Whether more pages follow.
    pub has_more: bool,
    /// Cursor of the next page.
    pub next_cursor: Option<String>,
}

/// Response of the health check.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Health {
    /// `ok` when healthy.
    pub status: String,
    /// Unix timestamp.
    pub time: i64,
}
