use std::sync::{Arc, Mutex};
use std::time::Duration;

use billionverify::*;
use serde_json::{json, Value};
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn envelope(data: Value) -> Value {
    json!({"success": true, "code": "0", "message": "Success", "data": data})
}

fn error_body(code: &str, message: &str) -> Value {
    json!({"success": false, "code": "4000", "message": "x", "error": {"code": code, "message": message}})
}

struct Fixture {
    server: MockServer,
    client: Client,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

async fn fixture() -> Fixture {
    let server = MockServer::start().await;
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let rec = sleeps.clone();
    let client = Client::builder()
        .api_key("test-key")
        .base_url(server.uri())
        .sleeper(move |d| {
            rec.lock().unwrap().push(d);
            async {}
        })
        .build()
        .unwrap();
    Fixture {
        server,
        client,
        sleeps,
    }
}

fn verification(email: &str) -> Value {
    json!({
        "email": email, "status": "valid", "score": 0.95, "is_deliverable": true,
        "is_disposable": false, "is_catchall": false, "is_role": false, "is_free": false,
        "domain": "example.com", "domain_age": 10, "mx_records": ["mail.example.com"],
        "check_smtp": true, "reason": "smtp_deliverable", "response_time": 250,
        "credits_used": 1, "verification_mode": "smtp", "retryable": false,
        "some_future_field": {"a": 1}
    })
}

#[tokio::test]
async fn verify_sends_key_and_only_set_options() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/single"))
        .and(header("BV-API-KEY", "test-key"))
        .and(header("Accept", "application/json"))
        .and(body_json(json!({"email": "user@example.com"})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(envelope(verification("user@example.com"))),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    let r = f
        .client
        .verify("user@example.com", Default::default())
        .await
        .unwrap();
    assert_eq!(r.status, "valid");
    assert_eq!(r.credits_used, 1);
    assert_eq!(r.mx_records.unwrap(), vec!["mail.example.com"]);
    let reqs = f.server.received_requests().await.unwrap();
    let ua = reqs[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(ua, format!("billionverify-rust/{VERSION}"));
}

#[tokio::test]
async fn verify_passes_options() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/single"))
        .and(body_json(json!({
            "email": "a@b.com", "check_smtp": false, "force_refresh": true,
            "include_domain_reputation": true
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(verification("a@b.com"))))
        .expect(1)
        .mount(&f.server)
        .await;
    f.client
        .verify(
            "a@b.com",
            VerifyOptions {
                check_smtp: Some(false),
                force_refresh: Some(true),
                include_domain_reputation: Some(true),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn verify_batch_posts_and_parses_buckets() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/bulk"))
        .and(body_json(json!({"emails": ["a@b.com", "c@d.com"]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "results": [verification("a@b.com"), verification("c@d.com")],
            "total_emails": 2, "valid_emails": 2, "invalid_emails": 0, "credits_used": 2,
            "process_time": 1500,
            "deliverability_buckets": {"strict_deliverable": 2, "usable_with_catchall": 2,
                "not_deliverable": 0, "unknown": 0, "anti_verified": 0, "transient_failure": 0}
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    let r = f
        .client
        .verify_batch(
            &["a@b.com".to_string(), "c@d.com".to_string()],
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(r.results.len(), 2);
    assert_eq!(r.deliverability_buckets.unwrap().strict_deliverable, 2);
}

#[tokio::test]
async fn verify_batch_rejects_more_than_50_locally() {
    let f = fixture().await;
    let emails: Vec<String> = (0..51).map(|i| format!("u{i}@example.com")).collect();
    let err = f
        .client
        .verify_batch(&emails, Default::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn check_disposable_post_and_get() {
    let f = fixture().await;
    let data = json!({"email": "u@mailinator.com", "domain": "mailinator.com", "is_disposable": true, "checked_at": "2026-01-01T00:00:00Z"});
    Mock::given(method("POST"))
        .and(path("/v1/verify/disposable"))
        .and(body_json(json!({"email": "u@mailinator.com"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(data.clone())))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/disposable"))
        .and(query_param("email", "u+tag@mailinator.com"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(data)))
        .expect(1)
        .mount(&f.server)
        .await;
    assert!(
        f.client
            .check_disposable("u@mailinator.com")
            .await
            .unwrap()
            .is_disposable
    );
    let r = f
        .client
        .check_disposable_via_query("u+tag@mailinator.com")
        .await
        .unwrap();
    assert_eq!(r.domain, "mailinator.com");
    let reqs = f.server.received_requests().await.unwrap();
    assert!(reqs[1]
        .url
        .query()
        .unwrap()
        .contains("email=u%2Btag%40mailinator.com"));
}

#[tokio::test]
async fn upload_file_sends_multipart_fields() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/file"))
        .and(header("BV-API-KEY", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "task_id": "job-1", "status": "pending", "message": "ok", "file_name": "emails.csv",
            "file_size": 12, "estimated_count": 2, "status_url": "/v1/verify/file/job-1",
            "created_at": "2026-01-01T00:00:00Z"
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    let r = f
        .client
        .upload_file(
            b"email\na@b.com\n".to_vec(),
            "emails.csv",
            UploadOptions {
                check_smtp: Some(false),
                email_column: Some("email".into()),
                preserve_original: Some(true),
            },
        )
        .await
        .unwrap();
    assert_eq!(r.task_id, "job-1");
    let reqs = f.server.received_requests().await.unwrap();
    let ct = reqs[0]
        .headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.starts_with("multipart/form-data"));
    let body = String::from_utf8_lossy(&reqs[0].body).to_string();
    assert!(body.contains("name=\"file\"; filename=\"emails.csv\""));
    assert!(body.contains("a@b.com"));
    assert!(body.contains("name=\"check_smtp\"\r\n\r\nfalse"));
    assert!(body.contains("name=\"email_column\"\r\n\r\nemail"));
    assert!(body.contains("name=\"preserve_original\"\r\n\r\ntrue"));
}

#[tokio::test]
async fn upload_file_path_reads_disk() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/file"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"task_id": "j", "status": "pending"}))),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    let dir = std::env::temp_dir().join("billionverify-rust-test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("list.txt");
    std::fs::write(&file, "a@b.com\n").unwrap();
    f.client
        .upload_file_path(&file, Default::default())
        .await
        .unwrap();
    let body =
        String::from_utf8_lossy(&f.server.received_requests().await.unwrap()[0].body).to_string();
    assert!(body.contains("filename=\"list.txt\""));
    let err = f
        .client
        .upload_file_path(dir.join("missing.csv"), Default::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
}

#[tokio::test]
async fn file_job_status_long_polling_and_range() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1"))
        .and(query_param("timeout", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "task_id": "job-1", "status": "completed", "progress": 100, "total_emails": 10,
            "result_download_available": true,
            "deliverability_buckets": {"strict_deliverable": 8}
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    let s = f
        .client
        .get_file_job_status("job-1", Some(20))
        .await
        .unwrap();
    assert_eq!(s.status, "completed");
    assert!(s.is_terminal());
    assert_eq!(s.deliverability_buckets.unwrap().strict_deliverable, 8);
    let err = f
        .client
        .get_file_job_status("job-1", Some(301))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
}

#[tokio::test]
async fn file_job_status_without_timeout_has_no_query() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"task_id": "job-1", "status": "running"}))),
        )
        .mount(&f.server)
        .await;
    f.client.get_file_job_status("job-1", None).await.unwrap();
    let reqs = f.server.received_requests().await.unwrap();
    assert_eq!(reqs[0].url.query(), None);
}

#[tokio::test]
async fn download_results_sends_filters() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1/results"))
        .and(query_param("valid", "true"))
        .and(query_param("risky", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string("email,status\na@b.com,valid\n"))
        .expect(1)
        .mount(&f.server)
        .await;
    let csv = f
        .client
        .download_file_results(
            "job-1",
            ResultFilters {
                valid: true,
                risky: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(csv).unwrap(),
        "email,status\na@b.com,valid\n"
    );
    let reqs = f.server.received_requests().await.unwrap();
    assert!(!reqs[0].url.query().unwrap().contains("invalid"));
}

#[tokio::test]
async fn download_results_redirect_does_not_leak_key() {
    let a = fixture().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/signed/results.csv"))
        .respond_with(ResponseTemplate::new(200).set_body_string("email,status\nx@y.com,valid\n"))
        .expect(1)
        .mount(&b)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1/results"))
        .and(header("BV-API-KEY", "test-key"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            format!("{}/signed/results.csv?sig=abc", b.uri()),
        ))
        .expect(1)
        .mount(&a.server)
        .await;
    let csv = a
        .client
        .download_file_results("job-1", Default::default())
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(csv).unwrap(),
        "email,status\nx@y.com,valid\n"
    );
    let reqs = b.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].headers.get("bv-api-key").is_none());
    assert!(reqs[0].headers.get("authorization").is_none());
    assert_eq!(reqs[0].url.query(), Some("sig=abc"));
}

#[tokio::test]
async fn download_results_redirect_error_from_storage_maps() {
    let a = fixture().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_string("denied"))
        .mount(&b)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1/results"))
        .respond_with(
            ResponseTemplate::new(307).insert_header("Location", format!("{}/x", b.uri())),
        )
        .mount(&a.server)
        .await;
    let err = a
        .client
        .download_file_results("job-1", Default::default())
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), 403);
}

#[tokio::test]
async fn resume_job_posts() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/jobs/job-1/resume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"task_id": "job-1", "status": "processing", "progress": 66}),
        )))
        .expect(1)
        .mount(&f.server)
        .await;
    let s = f.client.resume_job("job-1").await.unwrap();
    assert_eq!(s.status, "processing");
    assert_eq!(s.progress, 66);
}

#[tokio::test]
async fn wait_for_file_job_polls_until_terminal() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"task_id": "job-1", "status": "processing"}),
        )))
        .up_to_n_times(2)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/job-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"task_id": "job-1", "status": "completed_with_warning"}),
        )))
        .mount(&f.server)
        .await;
    let s = f
        .client
        .wait_for_file_job("job-1", Duration::from_secs(5), Duration::from_secs(600))
        .await
        .unwrap();
    assert_eq!(s.status, "completed_with_warning");
    assert_eq!(*f.sleeps.lock().unwrap(), vec![Duration::from_secs(5); 2]);
}

#[tokio::test]
async fn wait_for_file_job_stops_on_paused_and_times_out() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/paused"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"task_id": "paused", "status": "paused"}))),
        )
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/verify/file/stuck"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"task_id": "stuck", "status": "running"}))),
        )
        .mount(&f.server)
        .await;
    let s = f
        .client
        .wait_for_file_job("paused", Duration::from_secs(5), Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(s.status, "paused");
    let err = f
        .client
        .wait_for_file_job("stuck", Duration::from_secs(5), Duration::from_secs(20))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Timeout(_)));
}

#[tokio::test]
async fn credits_and_webhooks() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "account_id": "abc", "api_key_id": "k", "api_key_name": "Default",
            "credits_balance": 9500, "credits_consumed": 500, "credits_added": 10000,
            "last_updated": "2026-02-04T10:30:00Z"
        }))))
        .mount(&f.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/webhooks"))
        .and(body_json(
            json!({"url": "https://x.com/hook", "events": ["file.completed", "file.failed"]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "id": "w1", "url": "https://x.com/hook", "events": ["file.completed", "file.failed"],
            "secret": "s3cret", "is_active": true
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/webhooks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "webhooks": [{"id": "w1", "url": "https://x.com/hook", "events": ["file.completed"],
                "is_active": true, "last_delivery_status": "success", "last_error": null}],
            "total": 1
        }))))
        .mount(&f.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/webhooks/w1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(
            json!({"message": "Webhook deleted successfully", "webhook_id": "w1"}),
        )))
        .expect(1)
        .mount(&f.server)
        .await;

    assert_eq!(f.client.get_credits().await.unwrap().credits_balance, 9500);
    let w = f
        .client
        .create_webhook(
            "https://x.com/hook",
            &["file.completed".to_string(), "file.failed".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(w.secret.as_deref(), Some("s3cret"));
    let list = f.client.list_webhooks().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].last_delivery_status.as_deref(), Some("success"));
    assert!(list[0].secret.is_none());
    f.client.delete_webhook("w1").await.unwrap();
}

#[tokio::test]
async fn domain_search_flow() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/domain/search"))
        .and(body_json(json!({"domain": "example.com", "max_emails": 10, "include_role_based": false, "max_pages": 50})))
        .respond_with(ResponseTemplate::new(202).set_body_json(envelope(json!({
            "search_id": "s1", "status": "queued", "domain": "example.com",
            "credits_reserved": 50, "status_url": "/v1/domain/search/s1"
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/domain/search/s1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "search_id": "s1", "status": "completed", "domain": "example.com", "credits_charged": 15,
            "result": {"emails": [{"email": "a@example.com"}], "crawl_report": {"pages_fetched": 12}}
        }))))
        .mount(&f.server)
        .await;
    let job = f
        .client
        .create_domain_search(
            "example.com",
            10,
            DomainSearchOptions {
                include_role_based: Some(false),
                max_pages: Some(50),
            },
        )
        .await
        .unwrap();
    assert_eq!(job.credits_reserved, 50);
    let s = f
        .client
        .wait_for_domain_search("s1", Duration::from_secs(1), Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(s.credits_charged, 15);
    assert_eq!(s.result.unwrap()["crawl_report"]["pages_fetched"], 12);
}

#[tokio::test]
async fn domain_search_validates_max_emails_locally() {
    let f = fixture().await;
    for bad in [0u32, 201] {
        let err = f
            .client
            .create_domain_search("example.com", bad, Default::default())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }
    let err = f
        .client
        .create_domain_search_batch(
            b"domain\nexample.com\n".to_vec(),
            "d.csv",
            DomainSearchBatchOptions {
                max_emails: Some(500),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn domain_search_batch_flow() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/domain/search/batch"))
        .respond_with(ResponseTemplate::new(202).set_body_json(envelope(json!({
            "batch_id": "b1", "status": "queued", "total_rows": 2, "unique_domains": 2,
            "max_emails": 5, "include_role_based": true,
            "status_url": "/v1/domain/search/batch/b1", "results_url": "/v1/domain/search/batch/b1/results"
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/domain/search/batch"))
        .and(query_param("limit", "2"))
        .and(query_param("status", "completed"))
        .and(query_param("cursor", "c0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "items": [{"batch_id": "b1"}], "has_more": true, "next_cursor": "c1"
        }))))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/domain/search/batch/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({
            "batch_id": "b1", "status": "running", "max_emails": 5,
            "progress": {"total_rows": 2, "processed": 1, "found": 1, "emails_returned": 3, "unique_domains": 2},
            "credits_charged": 15
        }))))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/domain/search/batch/b1/results"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("row_no,domain,email\n1,a.com,x@a.com\n"),
        )
        .mount(&f.server)
        .await;

    let job = f
        .client
        .create_domain_search_batch(
            b"domain\na.com\nb.com\n".to_vec(),
            "domains.csv",
            DomainSearchBatchOptions {
                max_emails: Some(5),
                include_role_based: Some(true),
                max_pages: Some(20),
                column_map: Some(json!({"domain": "Company Website"})),
            },
        )
        .await
        .unwrap();
    assert_eq!(job.batch_id, "b1");
    let reqs = f.server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&reqs[0].body).to_string();
    assert!(body.contains("name=\"max_emails\"\r\n\r\n5"));
    assert!(body.contains("name=\"max_pages\"\r\n\r\n20"));
    assert!(body.contains("name=\"column_map\"\r\n\r\n{\"domain\":\"Company Website\"}"));

    let page = f
        .client
        .list_domain_search_batches(ListBatchesOptions {
            limit: Some(2),
            status: Some("completed".into()),
            cursor: Some("c0".into()),
        })
        .await
        .unwrap();
    assert!(page.has_more);
    assert_eq!(page.next_cursor.as_deref(), Some("c1"));
    let b = f.client.get_domain_search_batch("b1").await.unwrap();
    assert_eq!(b.progress.emails_returned, 3);
    let csv = f
        .client
        .download_domain_search_batch_results("b1")
        .await
        .unwrap();
    assert!(String::from_utf8(csv).unwrap().contains("x@a.com"));
}

#[tokio::test]
async fn batch_results_not_ready_is_409() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/domain/search/batch/b1/results"))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(error_body("BATCH_NOT_READY", "Results are not ready yet")),
        )
        .mount(&f.server)
        .await;
    let err = f
        .client
        .download_domain_search_batch_results("b1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Api(_)));
    assert_eq!(err.status_code(), 409);
    assert_eq!(err.code(), "BATCH_NOT_READY");
}

#[tokio::test]
async fn health_check_needs_no_key_and_no_envelope() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"status": "ok", "time": 1705319400})),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    let h = f.client.health_check().await.unwrap();
    assert_eq!(h.status, "ok");
    assert_eq!(h.time, 1705319400);
    let reqs = f.server.received_requests().await.unwrap();
    assert!(reqs[0].headers.get("bv-api-key").is_none());
}

#[tokio::test]
async fn errors_map_to_typed_variants() {
    let f = fixture().await;
    let cases: Vec<(u16, &str)> = vec![
        (400, "INVALID_REQUEST"),
        (401, "INVALID_API_KEY"),
        (402, "INSUFFICIENT_CREDITS"),
        (404, "JOB_NOT_FOUND"),
        (409, "CONFLICT"),
    ];
    for (status, code) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_json(error_body(code, "boom")))
            .mount(&server)
            .await;
        let client = Client::builder()
            .api_key("k")
            .base_url(server.uri())
            .build()
            .unwrap();
        let err = client.get_credits().await.unwrap_err();
        assert_eq!(err.status_code(), status);
        assert_eq!(err.code(), code);
        assert_eq!(err.message(), "boom");
        match (status, &err) {
            (400, Error::Validation(_))
            | (401, Error::Authentication(_))
            | (402, Error::InsufficientCredits(_))
            | (404, Error::NotFound(_))
            | (409, Error::Api(_)) => {}
            _ => panic!("status {status} mapped to {err:?}"),
        }
    }
    drop(f);
}

#[tokio::test]
async fn error_details_and_non_json_bodies() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "success": false, "error": {"code": "INVALID_REQUEST", "message": "bad", "details": {"field": "email"}}
        })))
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    let err = f.client.get_credits().await.unwrap_err();
    assert_eq!(err.details(), Some("{\"field\":\"email\"}"));

    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(ResponseTemplate::new(404).set_body_string("<html>nope</html>"))
        .mount(&f.server)
        .await;
    let err = f.client.get_credits().await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
    assert_eq!(err.code(), "UNKNOWN_ERROR");
    assert_eq!(err.message(), "Not Found");
}

#[tokio::test]
async fn rate_limit_retries_with_retry_after_then_succeeds() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "7")
                .set_body_json(error_body("RATE_LIMIT_EXCEEDED", "slow down")),
        )
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(envelope(json!({"credits_balance": 5}))),
        )
        .mount(&f.server)
        .await;
    let c = f.client.get_credits().await.unwrap();
    assert_eq!(c.credits_balance, 5);
    assert_eq!(*f.sleeps.lock().unwrap(), vec![Duration::from_secs(7)]);
}

#[tokio::test]
async fn rate_limit_exhausted_returns_rate_limit_error() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "3")
                .set_body_json(error_body("RATE_LIMIT_EXCEEDED", "slow down")),
        )
        .expect(3)
        .mount(&f.server)
        .await;
    let err = f.client.get_credits().await.unwrap_err();
    match err {
        Error::RateLimit { info, retry_after } => {
            assert_eq!(retry_after, 3);
            assert_eq!(info.status_code, 429);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn server_errors_back_off_exponentially_then_succeed() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(envelope(json!({"credits_balance": 1}))),
        )
        .mount(&f.server)
        .await;
    f.client.get_credits().await.unwrap();
    assert_eq!(
        *f.sleeps.lock().unwrap(),
        vec![Duration::from_secs(2), Duration::from_secs(4)]
    );
}

#[tokio::test]
async fn server_error_after_retries_is_api_error_and_400_is_not_retried() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(ResponseTemplate::new(500).set_body_json(error_body("INTERNAL", "oops")))
        .expect(3)
        .mount(&f.server)
        .await;
    let err = f.client.get_credits().await.unwrap_err();
    assert!(matches!(err, Error::Api(_)));
    assert_eq!(err.status_code(), 500);

    Mock::given(method("POST"))
        .and(path("/v1/verify/single"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(error_body("INVALID_REQUEST", "Invalid email format")),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    f.client
        .verify("bad", Default::default())
        .await
        .unwrap_err();
}

#[tokio::test]
async fn retry_resends_multipart_body() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/file"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/verify/file"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"task_id": "j", "status": "pending"}))),
        )
        .mount(&f.server)
        .await;
    f.client
        .upload_file(b"a@b.com\n".to_vec(), "l.txt", Default::default())
        .await
        .unwrap();
    let reqs = f.server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);
    assert!(String::from_utf8_lossy(&reqs[1].body).contains("a@b.com"));
}

#[tokio::test]
async fn timeout_is_a_timeout_error_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::builder()
        .api_key("k")
        .base_url(server.uri())
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let err = client.get_credits().await.unwrap_err();
    assert!(matches!(err, Error::Timeout(_)));
    assert_eq!(err.code(), "TIMEOUT");
    assert_eq!(err.status_code(), 504);
}

#[tokio::test]
async fn connection_refused_is_network_error() {
    let client = Client::builder()
        .api_key("k")
        .base_url("http://127.0.0.1:1")
        .build()
        .unwrap();
    let err = client.get_credits().await.unwrap_err();
    assert!(matches!(err, Error::Network(_)));
    assert_eq!(err.code(), "NETWORK_ERROR");
    assert_eq!(err.status_code(), 0);
}

#[tokio::test]
async fn empty_key_fails_before_any_request() {
    assert!(matches!(Client::new(""), Err(Error::Authentication(_))));
    assert!(matches!(Client::new("   "), Err(Error::Authentication(_))));
}

#[tokio::test]
async fn base_url_with_v1_and_trailing_slash_is_normalised() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .and(header_exists("BV-API-KEY"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(envelope(json!({"credits_balance": 2}))),
        )
        .expect(3)
        .mount(&server)
        .await;
    for suffix in ["/v1", "/v1/", "/"] {
        let client = Client::builder()
            .api_key("k")
            .base_url(format!("{}{}", server.uri(), suffix))
            .build()
            .unwrap();
        client.get_credits().await.unwrap();
    }
}

#[tokio::test]
async fn malformed_success_body_is_invalid_response() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/v1/credits"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&f.server)
        .await;
    let err = f.client.get_credits().await.unwrap_err();
    assert_eq!(err.code(), "INVALID_RESPONSE");
}

#[tokio::test]
async fn path_ids_are_percent_encoded() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(error_body("JOB_NOT_FOUND", "nope")))
        .mount(&f.server)
        .await;
    let _ = f.client.get_file_job_status("a/b c", None).await;
    let reqs = f.server.received_requests().await.unwrap();
    assert_eq!(reqs[0].url.path(), "/v1/verify/file/a%2Fb%20c");
}

mod webhook {
    use billionverify::verify_webhook_signature;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sign(secret: &str, ts: &str, body: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{ts}.{body}").as_bytes());
        let bytes = mac.finalize().into_bytes();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256={hex}")
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn accepts_valid_signature() {
        let ts = now().to_string();
        let body = r#"{"event":"file.completed"}"#;
        let sig = sign("secret", &ts, body);
        assert!(verify_webhook_signature(
            body.as_bytes(),
            &sig,
            &ts,
            "secret",
            300
        ));
    }

    #[test]
    fn rejects_tampered_body_wrong_secret_and_bad_format() {
        let ts = now().to_string();
        let body = r#"{"event":"file.completed"}"#;
        let sig = sign("secret", &ts, body);
        assert!(!verify_webhook_signature(
            b"{\"event\":\"file.failed\"}",
            &sig,
            &ts,
            "secret",
            300
        ));
        assert!(!verify_webhook_signature(
            body.as_bytes(),
            &sig,
            &ts,
            "other",
            300
        ));
        assert!(!verify_webhook_signature(
            body.as_bytes(),
            &sig.replace("sha256=", ""),
            &ts,
            "secret",
            300
        ));
        assert!(!verify_webhook_signature(
            body.as_bytes(),
            "sha256=",
            &ts,
            "secret",
            300
        ));
        assert!(!verify_webhook_signature(
            body.as_bytes(),
            &sig,
            "not-a-number",
            "secret",
            300
        ));
    }

    #[test]
    fn rejects_old_timestamp_unless_tolerance_is_zero() {
        let ts = (now() - 3600).to_string();
        let body = "{}";
        let sig = sign("secret", &ts, body);
        assert!(!verify_webhook_signature(
            body.as_bytes(),
            &sig,
            &ts,
            "secret",
            300
        ));
        assert!(verify_webhook_signature(
            body.as_bytes(),
            &sig,
            &ts,
            "secret",
            0
        ));
    }

    #[test]
    fn signature_over_body_alone_is_rejected() {
        // Signing only the body (without the timestamp prefix) must not verify.
        let ts = now().to_string();
        let body = "{}";
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(body.as_bytes());
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(!verify_webhook_signature(
            body.as_bytes(),
            &format!("sha256={hex}"),
            &ts,
            "secret",
            300
        ));
    }
}
