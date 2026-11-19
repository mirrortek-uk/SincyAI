//! Unified error model (Guide 3.2; TDD 12.1 unified JSON error structure).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Classification of an application error. Drives retry / degrade / human decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Transient failure (network, timeout, 5xx). Safe to retry with bounded backoff.
    Retryable,
    /// Permanent failure. Must not be retried.
    NonRetryable,
    /// Requires human action (captcha/challenge, consent, approval, WAITING_USER).
    NeedsUser,
    /// Failed before any external side effect. Eligible for route degradation.
    NoSideEffect,
    /// Side effect may have occurred. Must NOT retry, degrade, or blindly re-execute.
    Uncertain,
}

impl ErrorKind {
    pub fn http_status(&self) -> u16 {
        match self {
            ErrorKind::Retryable => 503,
            ErrorKind::NonRetryable => 400,
            ErrorKind::NeedsUser => 409,
            ErrorKind::NoSideEffect => 502,
            ErrorKind::Uncertain => 409,
        }
    }
}

/// Application-wide error. Never carries secrets.
#[derive(Debug, Error)]
pub struct AppError {
    pub kind: ErrorKind,
    pub code: &'static str,
    pub message: String,
    /// Optional correlation id; filled by middleware when absent.
    pub request_id: Option<String>,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl AppError {
    pub fn new(kind: ErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            request_id: None,
            source: None,
        }
    }

    pub fn retryable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Retryable, code, message)
    }

    pub fn non_retryable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NonRetryable, code, message)
    }

    pub fn needs_user(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NeedsUser, code, message)
    }

    pub fn no_side_effect(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NoSideEffect, code, message)
    }

    pub fn uncertain(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Uncertain, code, message)
    }

    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// JSON body returned to API clients (TDD 12.1: unified error structure + request_id).
    pub fn to_body(&self) -> ErrorBody {
        ErrorBody {
            error: ErrorBodyError {
                code: std::borrow::Cow::Borrowed(self.code),
                kind: self.kind,
                message: self.message.clone(),
                request_id: self.request_id.clone().unwrap_or_default(),
            },
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}: {}", self.kind, self.code, self.message)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        let kind = if e.kind() == std::io::ErrorKind::TimedOut {
            ErrorKind::Retryable
        } else {
            ErrorKind::NonRetryable
        };
        AppError::new(kind, "io_error", e.to_string()).with_source(e)
    }
}

/// Serializable error envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorBodyError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBodyError {
    pub code: std::borrow::Cow<'static, str>,
    pub kind: ErrorKind,
    pub message: String,
    pub request_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_body_is_unified_shape() {
        let err =
            AppError::needs_user("waiting_user", "challenge detected").with_request_id("req-1");
        let body = err.to_body();
        let json = serde_json::to_value(&body).expect("serialize");
        assert_eq!(json["error"]["code"], "waiting_user");
        assert_eq!(json["error"]["kind"], "needs_user");
        assert_eq!(json["error"]["request_id"], "req-1");
        assert_eq!(ErrorKind::NeedsUser.http_status(), 409);
    }

    #[test]
    fn uncertain_never_retryable() {
        let err = AppError::uncertain("verification_pending", "side effect may have occurred");
        assert_eq!(err.kind, ErrorKind::Uncertain);
        assert_ne!(err.kind, ErrorKind::Retryable);
        assert_ne!(err.kind, ErrorKind::NoSideEffect);
    }
}
