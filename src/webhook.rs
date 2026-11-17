use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

/// Verify the signature of a webhook delivery.
///
/// * `raw_body` - the request body exactly as received.
/// * `signature` - the `X-Webhook-Signature` header (`sha256=<hex>`).
/// * `timestamp` - the `X-Webhook-Timestamp` header (Unix seconds).
/// * `secret` - the secret returned when the webhook was created.
/// * `tolerance_seconds` - how old a timestamp may be (and how far in the future); `0` skips the
///   check. Five minutes (`300`) is a good default.
pub fn verify_webhook_signature(
    raw_body: &[u8],
    signature: &str,
    timestamp: &str,
    secret: &str,
    tolerance_seconds: u64,
) -> bool {
    let ts: i64 = match timestamp.trim().parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if tolerance_seconds > 0 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if (now - ts).unsigned_abs() > tolerance_seconds {
            return false;
        }
    }
    let provided = match signature.trim().strip_prefix("sha256=") {
        Some(h) => h,
        None => return false,
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(timestamp.trim().as_bytes());
    mac.update(b".");
    mac.update(raw_body);
    let expected = hex(&mac.finalize().into_bytes());
    constant_time_eq(
        expected.as_bytes(),
        provided.to_ascii_lowercase().as_bytes(),
    )
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
