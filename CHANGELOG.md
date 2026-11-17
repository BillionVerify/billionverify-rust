# Changelog

## 1.0.0

- First release.
- Async client covering every BillionVerify API operation: single and batch verification, disposable check (POST and GET), file verification (upload, status with long polling, results download, resume), credits, webhooks, domain search, batch domain search and health check.
- Typed errors with automatic retries on 429, 500, 502 and 503.
- Webhook signature verification over `{timestamp}.{raw_body}` with a replay window.
