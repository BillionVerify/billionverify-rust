use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::error::{Error, ErrorInfo, Result};
use crate::types::*;
use crate::VERSION;

/// Default API host.
pub const DEFAULT_BASE_URL: &str = "https://api.billionverify.com";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_RETRIES: u32 = 3;
const MAX_REDIRECTS: usize = 5;

type Sleeper = Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Builds a [`Client`].
pub struct ClientBuilder {
    api_key: String,
    base_url: String,
    timeout: Duration,
    retries: u32,
    sleeper: Option<Sleeper>,
}

impl ClientBuilder {
    /// Your API key (required).
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = key.into();
        self
    }

    /// API host. A trailing `/` and a trailing `/v1` are ignored.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Request timeout (default 30 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Total attempts per request, first one included (default 3). Requests are retried on
    /// 429, 500, 502 and 503.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Replace the function that waits between retries and polls (tests use this to skip waits).
    pub fn sleeper<F, Fut>(mut self, sleeper: F) -> Self
    where
        F: Fn(Duration) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.sleeper = Some(Arc::new(move |d| Box::pin(sleeper(d))));
        self
    }

    /// Build the client. Fails with [`Error::Authentication`] when the API key is empty.
    pub fn build(self) -> Result<Client> {
        if self.api_key.trim().is_empty() {
            return Err(Error::authentication("API key is required"));
        }
        let mut base = self.base_url.trim().trim_end_matches('/').to_string();
        if let Some(stripped) = base.strip_suffix("/v1") {
            base = stripped.trim_end_matches('/').to_string();
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("billionverify-rust/{VERSION}"))
            .build()
            .map_err(|e| {
                Error::Network(ErrorInfo::new(
                    e.without_url().to_string(),
                    "NETWORK_ERROR",
                    0,
                ))
            })?;
        let sleeper: Sleeper = self
            .sleeper
            .unwrap_or_else(|| Arc::new(|d| Box::pin(async move { tokio::time::sleep(d).await })));
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                api_key: self.api_key,
                base_url: base,
                timeout: self.timeout,
                retries: self.retries.max(1),
                sleeper,
            }),
        })
    }
}

struct Inner {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    timeout: Duration,
    retries: u32,
    sleeper: Sleeper,
}

/// BillionVerify API client. Cheap to clone; clones share one connection pool.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

enum Body {
    None,
    Json(Value),
    Multipart(Vec<(&'static str, Field)>),
}

enum Field {
    Text(String),
    File { bytes: Vec<u8>, filename: String },
}

fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn guess_mime(filename: &str) -> &'static str {
    match filename
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv") => "text/csv",
        Some("txt") => "text/plain",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

fn map_error(status: StatusCode, retry_after: u64, body: &[u8]) -> Error {
    let code_num = status.as_u16();
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let mut message = status
        .canonical_reason()
        .unwrap_or("Unknown error")
        .to_string();
    let mut code = "UNKNOWN_ERROR".to_string();
    let mut details = None;
    if let Some(Value::Object(obj)) = &parsed {
        if let Some(m) = obj.get("message").and_then(Value::as_str) {
            if !m.is_empty() {
                message = m.to_string();
            }
        }
        if let Some(Value::Object(err)) = obj.get("error") {
            if let Some(m) = err.get("message").and_then(Value::as_str) {
                if !m.is_empty() {
                    message = m.to_string();
                }
            }
            if let Some(c) = err.get("code").and_then(Value::as_str) {
                if !c.is_empty() {
                    code = c.to_string();
                }
            }
            details = match err.get("details") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(other) => Some(other.to_string()),
            };
        }
    }
    let mut info = ErrorInfo {
        message,
        code,
        status_code: code_num,
        details,
    };
    match code_num {
        400 => Error::Validation(info),
        401 => Error::Authentication(info),
        402 => Error::InsufficientCredits(info),
        404 => Error::NotFound(info),
        429 => Error::RateLimit { info, retry_after },
        _ => {
            if info.code == "UNKNOWN_ERROR" && code_num >= 500 {
                info.code = "SERVER_ERROR".to_string();
            }
            Error::Api(info)
        }
    }
}

fn map_transport(e: reqwest::Error, timeout: Duration) -> Error {
    if e.is_timeout() {
        Error::timeout(format!("Request timed out after {}ms", timeout.as_millis()))
    } else {
        Error::Network(ErrorInfo::new(
            format!("Network error: {}", e.without_url()),
            "NETWORK_ERROR",
            0,
        ))
    }
}

impl Client {
    /// A client with default settings.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::builder().api_key(api_key).build()
    }

    /// Start building a client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            api_key: String::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            retries: DEFAULT_RETRIES,
            sleeper: None,
        }
    }

    async fn sleep(&self, d: Duration) {
        (self.inner.sleeper)(d).await
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> Result<Url> {
        let full = format!("{}{}", self.inner.base_url, path);
        let parsed = if query.is_empty() {
            Url::parse(&full)
        } else {
            Url::parse_with_params(&full, query)
        };
        parsed.map_err(|e| Error::validation(format!("invalid base URL: {e}")))
    }

    /// Send a request with retries. Returns the response on 2xx (and on 3xx when
    /// `keep_redirect` is set); maps everything else to an error.
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Body,
        auth: bool,
        timeout: Option<Duration>,
        keep_redirect: bool,
    ) -> Result<reqwest::Response> {
        let url = self.url(path, query)?;
        let timeout = timeout.unwrap_or(self.inner.timeout);
        let mut attempt: u32 = 1;
        loop {
            let mut req = self
                .inner
                .http
                .request(method.clone(), url.clone())
                .header("Accept", "application/json")
                .timeout(timeout);
            if auth {
                req = req.header("BV-API-KEY", &self.inner.api_key);
            }
            req = match &body {
                Body::None => req,
                Body::Json(v) => req.json(v),
                Body::Multipart(fields) => {
                    let mut form = Form::new();
                    for (name, field) in fields {
                        form = match field {
                            Field::Text(t) => form.text(*name, t.clone()),
                            Field::File { bytes, filename } => {
                                let part = Part::bytes(bytes.clone())
                                    .file_name(filename.clone())
                                    .mime_str(guess_mime(filename))
                                    .map_err(|e| Error::validation(e.to_string()))?;
                                form.part(*name, part)
                            }
                        };
                    }
                    req.multipart(form)
                }
            };
            let resp = req.send().await.map_err(|e| map_transport(e, timeout))?;
            let status = resp.status();
            if status.is_success() || (keep_redirect && status.is_redirection()) {
                return Ok(resp);
            }
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let bytes = resp.bytes().await.map_err(|e| map_transport(e, timeout))?;
            let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503);
            if retryable && attempt < self.inner.retries {
                let wait = if status.as_u16() == 429 && retry_after > 0 {
                    retry_after
                } else {
                    2u64.saturating_pow(attempt)
                };
                self.sleep(Duration::from_secs(wait)).await;
                attempt += 1;
                continue;
            }
            return Err(map_error(status, retry_after, &bytes));
        }
    }

    /// Send a request and return the envelope's `data`.
    async fn call_value(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Body,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let resp = self
            .execute(method, path, query, body, true, timeout, false)
            .await?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| map_transport(e, timeout.unwrap_or(self.inner.timeout)))?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| {
            Error::invalid_response(format!("invalid JSON in response from {path}: {e}"), status)
        })?;
        match value {
            Value::Object(mut obj) if obj.contains_key("data") => {
                Ok(obj.remove("data").unwrap_or(Value::Null))
            }
            other => Ok(other),
        }
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Body,
        timeout: Option<Duration>,
    ) -> Result<T> {
        let data = self.call_value(method, path, query, body, timeout).await?;
        if data.is_null() {
            return Err(Error::invalid_response(
                format!("empty response from {path}"),
                200,
            ));
        }
        serde_json::from_value(data).map_err(|e| {
            Error::invalid_response(format!("malformed response from {path}: {e}"), 200)
        })
    }

    /// Download a body, following redirects by hand: the redirect target is a different host
    /// (pre-signed storage), so the API key must not travel with it.
    async fn download(&self, path: &str, query: &[(&str, String)]) -> Result<Vec<u8>> {
        let mut resp = self
            .execute(Method::GET, path, query, Body::None, true, None, true)
            .await?;
        let mut hops = 0;
        while resp.status().is_redirection() {
            hops += 1;
            if hops > MAX_REDIRECTS {
                return Err(Error::Api(ErrorInfo::new(
                    "too many redirects",
                    "TOO_MANY_REDIRECTS",
                    resp.status().as_u16(),
                )));
            }
            let location = resp
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| {
                    Error::invalid_response("redirect without Location", resp.status().as_u16())
                })?;
            let next = resp
                .url()
                .join(&location)
                .map_err(|e| Error::invalid_response(format!("bad redirect: {e}"), 307))?;
            // No BV-API-KEY on the second hop.
            let r = self
                .inner
                .http
                .get(next)
                .timeout(self.inner.timeout)
                .send()
                .await
                .map_err(|e| map_transport(e, self.inner.timeout))?;
            if !r.status().is_success() && !r.status().is_redirection() {
                let status = r.status();
                let bytes = r
                    .bytes()
                    .await
                    .map_err(|e| map_transport(e, self.inner.timeout))?;
                return Err(map_error(status, 0, &bytes));
            }
            resp = r;
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| map_transport(e, self.inner.timeout))?;
        Ok(bytes.to_vec())
    }

    // ---- verification -------------------------------------------------------------------

    /// Verify one address (`POST /v1/verify/single`).
    pub async fn verify(&self, email: &str, options: VerifyOptions) -> Result<VerificationResult> {
        let mut body = Map::new();
        body.insert("email".into(), json!(email));
        apply_verify_options(&mut body, &options);
        self.call(
            Method::POST,
            "/v1/verify/single",
            &[],
            Body::Json(Value::Object(body)),
            None,
        )
        .await
    }

    /// Verify up to 50 addresses in one synchronous request (`POST /v1/verify/bulk`).
    pub async fn verify_batch(
        &self,
        emails: &[String],
        options: VerifyOptions,
    ) -> Result<BatchVerificationResult> {
        if emails.len() > 50 {
            return Err(Error::validation(
                "Maximum 50 emails per batch request. For larger lists, use upload_file().",
            ));
        }
        let mut body = Map::new();
        body.insert("emails".into(), json!(emails));
        apply_verify_options(&mut body, &options);
        self.call(
            Method::POST,
            "/v1/verify/bulk",
            &[],
            Body::Json(Value::Object(body)),
            None,
        )
        .await
    }

    /// Free disposable-domain check (`POST /v1/verify/disposable`).
    pub async fn check_disposable(&self, email: &str) -> Result<DisposableCheck> {
        self.call(
            Method::POST,
            "/v1/verify/disposable",
            &[],
            Body::Json(json!({ "email": email })),
            None,
        )
        .await
    }

    /// Free disposable-domain check, query-string variant (`GET /v1/verify/disposable`).
    pub async fn check_disposable_via_query(&self, email: &str) -> Result<DisposableCheck> {
        self.call(
            Method::GET,
            "/v1/verify/disposable",
            &[("email", email.to_string())],
            Body::None,
            None,
        )
        .await
    }

    // ---- file jobs ----------------------------------------------------------------------

    /// Upload a CSV, XLSX or TXT file for asynchronous verification (`POST /v1/verify/file`).
    pub async fn upload_file(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        options: UploadOptions,
    ) -> Result<FileUpload> {
        let mut fields = vec![(
            "file",
            Field::File {
                bytes,
                filename: filename.to_string(),
            },
        )];
        if let Some(v) = options.check_smtp {
            fields.push(("check_smtp", Field::Text(v.to_string())));
        }
        if let Some(v) = options.email_column {
            fields.push(("email_column", Field::Text(v)));
        }
        if let Some(v) = options.preserve_original {
            fields.push(("preserve_original", Field::Text(v.to_string())));
        }
        self.call(
            Method::POST,
            "/v1/verify/file",
            &[],
            Body::Multipart(fields),
            None,
        )
        .await
    }

    /// Upload a file from disk.
    pub async fn upload_file_path(
        &self,
        path: impl AsRef<std::path::Path>,
        options: UploadOptions,
    ) -> Result<FileUpload> {
        let path = path.as_ref();
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| Error::validation(format!("cannot read {}: {e}", path.display())))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upload.csv")
            .to_string();
        self.upload_file(bytes, &name, options).await
    }

    /// Status of a file job (`GET /v1/verify/file/{job_id}`). `timeout_seconds` (0-300) makes the
    /// server hold the request until the job finishes or the time is up.
    pub async fn get_file_job_status(
        &self,
        job_id: &str,
        timeout_seconds: Option<u32>,
    ) -> Result<FileJobStatus> {
        let mut query = Vec::new();
        let mut timeout = None;
        if let Some(t) = timeout_seconds {
            if t > 300 {
                return Err(Error::validation(
                    "Timeout must be between 0 and 300 seconds",
                ));
            }
            if t > 0 {
                query.push(("timeout", t.to_string()));
                timeout = Some(Duration::from_secs(u64::from(t) + 10));
            }
        }
        self.call(
            Method::GET,
            &format!("/v1/verify/file/{}", encode_segment(job_id)),
            &query,
            Body::None,
            timeout,
        )
        .await
    }

    /// Download the result CSV of a finished job (`GET /v1/verify/file/{job_id}/results`).
    pub async fn download_file_results(
        &self,
        job_id: &str,
        filters: ResultFilters,
    ) -> Result<Vec<u8>> {
        let mut query = Vec::new();
        for (name, on) in [
            ("valid", filters.valid),
            ("invalid", filters.invalid),
            ("catchall", filters.catchall),
            ("role", filters.role),
            ("unknown", filters.unknown),
            ("disposable", filters.disposable),
            ("risky", filters.risky),
        ] {
            if on {
                query.push((name, "true".to_string()));
            }
        }
        self.download(
            &format!("/v1/verify/file/{}/results", encode_segment(job_id)),
            &query,
        )
        .await
    }

    /// Resume a paused file job (`POST /v1/jobs/{job_id}/resume`).
    pub async fn resume_job(&self, job_id: &str) -> Result<FileJobStatus> {
        self.call(
            Method::POST,
            &format!("/v1/jobs/{}/resume", encode_segment(job_id)),
            &[],
            Body::None,
            None,
        )
        .await
    }

    /// Poll a file job until it finishes (or is paused, which never changes by itself).
    /// Fails with [`Error::Timeout`] after `max_wait`.
    pub async fn wait_for_file_job(
        &self,
        job_id: &str,
        poll_interval: Duration,
        max_wait: Duration,
    ) -> Result<FileJobStatus> {
        let start = Instant::now();
        let mut waited = Duration::ZERO;
        loop {
            let status = self.get_file_job_status(job_id, None).await?;
            if status.is_terminal() || status.status == "paused" {
                return Ok(status);
            }
            if start.elapsed().max(waited) >= max_wait {
                return Err(Error::timeout(format!(
                    "File job {job_id} did not complete within {}ms",
                    max_wait.as_millis()
                )));
            }
            self.sleep(poll_interval).await;
            waited += poll_interval;
        }
    }

    // ---- account and webhooks -----------------------------------------------------------

    /// Credit balance (`GET /v1/credits`).
    pub async fn get_credits(&self) -> Result<Credits> {
        self.call(Method::GET, "/v1/credits", &[], Body::None, None)
            .await
    }

    /// Create a webhook (`POST /v1/webhooks`). `events` are `file.completed` and `file.failed`;
    /// the URL must be HTTPS. The secret is returned only here.
    pub async fn create_webhook(&self, url: &str, events: &[String]) -> Result<Webhook> {
        self.call(
            Method::POST,
            "/v1/webhooks",
            &[],
            Body::Json(json!({ "url": url, "events": events })),
            None,
        )
        .await
    }

    /// List webhooks (`GET /v1/webhooks`). Secrets are not included.
    pub async fn list_webhooks(&self) -> Result<Vec<Webhook>> {
        let data = self
            .call_value(Method::GET, "/v1/webhooks", &[], Body::None, None)
            .await?;
        let list = match data {
            Value::Object(mut o) => o.remove("webhooks").unwrap_or(Value::Null),
            other => other,
        };
        if list.is_null() {
            return Ok(Vec::new());
        }
        serde_json::from_value(list).map_err(|e| {
            Error::invalid_response(format!("malformed response from /v1/webhooks: {e}"), 200)
        })
    }

    /// Delete a webhook (`DELETE /v1/webhooks/{webhook_id}`).
    pub async fn delete_webhook(&self, webhook_id: &str) -> Result<()> {
        self.call_value(
            Method::DELETE,
            &format!("/v1/webhooks/{}", encode_segment(webhook_id)),
            &[],
            Body::None,
            None,
        )
        .await
        .map(|_| ())
    }

    // ---- domain search ------------------------------------------------------------------

    /// Start a domain search (`POST /v1/domain/search`). `max_emails` is 1-200 and caps the bill.
    pub async fn create_domain_search(
        &self,
        domain: &str,
        max_emails: u32,
        options: DomainSearchOptions,
    ) -> Result<DomainSearchJob> {
        check_max_emails(max_emails)?;
        let mut body = Map::new();
        body.insert("domain".into(), json!(domain));
        body.insert("max_emails".into(), json!(max_emails));
        if let Some(v) = options.include_role_based {
            body.insert("include_role_based".into(), json!(v));
        }
        if let Some(v) = options.max_pages {
            body.insert("max_pages".into(), json!(v));
        }
        self.call(
            Method::POST,
            "/v1/domain/search",
            &[],
            Body::Json(Value::Object(body)),
            None,
        )
        .await
    }

    /// Status and result of a domain search (`GET /v1/domain/search/{search_id}`).
    pub async fn get_domain_search(&self, search_id: &str) -> Result<DomainSearch> {
        self.call(
            Method::GET,
            &format!("/v1/domain/search/{}", encode_segment(search_id)),
            &[],
            Body::None,
            None,
        )
        .await
    }

    /// Poll a domain search until it finishes. Fails with [`Error::Timeout`] after `max_wait`.
    pub async fn wait_for_domain_search(
        &self,
        search_id: &str,
        poll_interval: Duration,
        max_wait: Duration,
    ) -> Result<DomainSearch> {
        let start = Instant::now();
        let mut waited = Duration::ZERO;
        loop {
            let search = self.get_domain_search(search_id).await?;
            if search.is_terminal() {
                return Ok(search);
            }
            if start.elapsed().max(waited) >= max_wait {
                return Err(Error::timeout(format!(
                    "Domain search {search_id} did not complete within {}ms",
                    max_wait.as_millis()
                )));
            }
            self.sleep(poll_interval).await;
            waited += poll_interval;
        }
    }

    /// Start a batch domain search from a CSV of domains (`POST /v1/domain/search/batch`).
    pub async fn create_domain_search_batch(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        options: DomainSearchBatchOptions,
    ) -> Result<DomainSearchBatchJob> {
        if let Some(m) = options.max_emails {
            check_max_emails(m)?;
        }
        let mut fields = vec![(
            "file",
            Field::File {
                bytes,
                filename: filename.to_string(),
            },
        )];
        if let Some(v) = options.max_emails {
            fields.push(("max_emails", Field::Text(v.to_string())));
        }
        if let Some(v) = options.include_role_based {
            fields.push(("include_role_based", Field::Text(v.to_string())));
        }
        if let Some(v) = options.max_pages {
            fields.push(("max_pages", Field::Text(v.to_string())));
        }
        if let Some(v) = options.column_map {
            let text = match v {
                Value::String(s) => s,
                other => other.to_string(),
            };
            fields.push(("column_map", Field::Text(text)));
        }
        self.call(
            Method::POST,
            "/v1/domain/search/batch",
            &[],
            Body::Multipart(fields),
            None,
        )
        .await
    }

    /// List batch domain searches (`GET /v1/domain/search/batch`).
    pub async fn list_domain_search_batches(
        &self,
        options: ListBatchesOptions,
    ) -> Result<DomainSearchBatchPage> {
        let mut query = Vec::new();
        if let Some(v) = options.limit {
            query.push(("limit", v.to_string()));
        }
        if let Some(v) = options.status {
            query.push(("status", v));
        }
        if let Some(v) = options.cursor {
            query.push(("cursor", v));
        }
        self.call(
            Method::GET,
            "/v1/domain/search/batch",
            &query,
            Body::None,
            None,
        )
        .await
    }

    /// Status of a batch domain search (`GET /v1/domain/search/batch/{batch_id}`).
    pub async fn get_domain_search_batch(&self, batch_id: &str) -> Result<DomainSearchBatch> {
        self.call(
            Method::GET,
            &format!("/v1/domain/search/batch/{}", encode_segment(batch_id)),
            &[],
            Body::None,
            None,
        )
        .await
    }

    /// Download the results CSV of a finished batch
    /// (`GET /v1/domain/search/batch/{batch_id}/results`). Returns a 409 [`Error::Api`] until the
    /// batch has finished.
    pub async fn download_domain_search_batch_results(&self, batch_id: &str) -> Result<Vec<u8>> {
        self.download(
            &format!(
                "/v1/domain/search/batch/{}/results",
                encode_segment(batch_id)
            ),
            &[],
        )
        .await
    }

    // ---- health -------------------------------------------------------------------------

    /// Health check (`GET /health`, no authentication).
    pub async fn health_check(&self) -> Result<Health> {
        let resp = self
            .execute(Method::GET, "/health", &[], Body::None, false, None, false)
            .await?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| map_transport(e, self.inner.timeout))?;
        serde_json::from_slice(&bytes).map_err(|e| {
            Error::invalid_response(format!("malformed response from /health: {e}"), status)
        })
    }
}

fn apply_verify_options(body: &mut Map<String, Value>, o: &VerifyOptions) {
    if let Some(v) = o.check_smtp {
        body.insert("check_smtp".into(), json!(v));
    }
    if let Some(v) = o.force_refresh {
        body.insert("force_refresh".into(), json!(v));
    }
    if let Some(v) = o.include_domain_reputation {
        body.insert("include_domain_reputation".into(), json!(v));
    }
}

fn check_max_emails(v: u32) -> Result<()> {
    if !(1..=200).contains(&v) {
        return Err(Error::validation("max_emails must be between 1 and 200"));
    }
    Ok(())
}
