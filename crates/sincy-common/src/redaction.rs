//! Log redaction (WP-15 / TDD security: 日志脱敏).
//!
//! Secrets must never land in logs or error bodies. `redact` masks values of
//! sensitive keys (token, secret, password, authorization, ...) and long
//! opaque credential-like runs (hex/base64 >= 16 chars), in any log line.

/// Keys whose value gets fully masked when found as `key=value` / `"key": "value"`.
const SENSITIVE_KEYS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "authorization",
    "auth",
    "api_key",
    "apikey",
    "access_key",
    "refresh_token",
    "session",
    "proof",
    "nonce",
];

/// Redact a single log line. Keeps structure, replaces secret material with ***.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // match key = value or key: value or "key":"value"
        let ch = chars[i];
        // quoted key form can start at a '"' anywhere (e.g. inside JSON)
        if ch == '"' {
            let rest: String = chars[i..].iter().collect();
            if let Some((key, consumed, value_span)) = match_kv(&rest)
                && SENSITIVE_KEYS
                    .iter()
                    .any(|k| key.to_lowercase().contains(k))
            {
                out.push_str(&rest[..consumed]);
                out.push_str("***");
                i += consumed + value_span;
                continue;
            }
        }
        let rest: String = chars[i..].iter().collect();
        if let Some((key, consumed, value_span)) = match_kv(&rest) {
            let key_lc = key.to_lowercase();
            if SENSITIVE_KEYS.iter().any(|k| key_lc.contains(k)) {
                out.push_str(&rest[..consumed]);
                out.push_str("***");
                i += consumed + value_span;
                continue;
            }
            out.push_str(&rest[..consumed + value_span]);
            i += consumed + value_span;
            continue;
        }
        // bare credential-like run: >=16 of [A-Za-z0-9_\-+/=.] starting with alnum
        if ch.is_alphanumeric() {
            let start = i;
            while i < chars.len() && is_token_char(chars[i]) {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            if run.len() >= 16 && looks_like_credential(&run) {
                out.push_str("***");
            } else {
                out.push_str(&run);
            }
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Try to parse `key=value`, `key: value`, `"key": "value"` at the start of s.
/// Returns (key, prefix_len_including_separator, value_len).
fn match_kv(s: &str) -> Option<(String, usize, usize)> {
    let b = s.as_bytes();
    // quoted key form: "key": "value" or "key":"value"
    if b.first() == Some(&b'"') {
        if let Some(end) = s[1..].find('"').map(|e| e + 1) {
            let key = &s[1..end];
            let mut j = end + 1;
            while j < b.len() && (b[j] as char).is_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == b':' {
                j += 1;
                while j < b.len() && (b[j] as char).is_whitespace() {
                    j += 1;
                }
                if j < b.len()
                    && b[j] == b'"'
                    && let Some(ve) = s[j + 1..].find('"').map(|e| e + j + 1)
                {
                    // prefix up to the opening quote; value span covers both quotes
                    return Some((key.to_string(), j, ve - j + 1));
                }
            }
        }
        return None;
    }
    // bare key form
    let mut j = 0;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'-') {
        j += 1;
    }
    if j == 0 {
        return None;
    }
    let key = &s[..j];
    let mut k = j;
    while k < b.len() && (b[k] as char).is_whitespace() {
        k += 1;
    }
    if k < b.len() && (b[k] == b'=' || b[k] == b':') {
        k += 1;
        while k < b.len() && (b[k] as char).is_whitespace() {
            k += 1;
        }
        // value: until whitespace, comma, or closing quote/brace
        let vstart = k;
        while k < b.len()
            && !(b[k] as char).is_whitespace()
            && b[k] != b','
            && b[k] != b'}'
            && b[k] != b'"'
        {
            k += 1;
        }
        if k > vstart {
            return Some((key.to_string(), vstart, k - vstart));
        }
    }
    None
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.' | ':')
}

/// Heuristic: mixed-case alnum + symbols long enough to be a credential, not a word.
fn looks_like_credential(run: &str) -> bool {
    let has_upper = run.chars().any(|c| c.is_uppercase());
    let has_lower = run.chars().any(|c| c.is_lowercase());
    let has_digit = run.chars().any(|c| c.is_numeric());
    let has_symbol = run
        .chars()
        .any(|c| matches!(c, '-' | '_' | '+' | '/' | '=' | '.'));
    // JWT-ish or hex-ish or base64-ish
    (has_upper && has_lower && has_digit)
        || (run.len() >= 32 && (has_digit || has_symbol) && has_lower)
        || run.split('.').count() == 3 && run.len() >= 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_sensitive_keys() {
        assert_eq!(redact("token=abc123xyz"), "token=***");
        assert_eq!(
            redact("Authorization: Bearer abcdef0123456789ABCDEF"),
            "Authorization: *** ***"
        );
        assert_eq!(
            redact(r#"{"secret": "s3cr3t-value"}"#),
            r#"{"secret": ***}"#
        );
        assert_eq!(redact("password=hunter2!"), "password=***");
    }

    #[test]
    fn masks_bare_long_tokens() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let r = redact(&format!("got {jwt} back"));
        assert!(!r.contains(jwt), "jwt leaked: {r}");
        assert!(r.contains("***"));
        let hex = "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3";
        let r = redact(&format!("hash {hex} end"));
        assert!(!r.contains(hex));
    }

    #[test]
    fn keeps_normal_text() {
        assert_eq!(
            redact("task 123 published to youtube"),
            "task 123 published to youtube"
        );
        assert_eq!(redact("agency=ok state=QUEUED"), "agency=ok state=QUEUED");
    }
}
