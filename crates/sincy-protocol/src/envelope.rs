//! Protobuf Envelope semantics (TDD 12.1):
//! message_id, correlation_id, sent_at, schema_version, tenant_id, device_id.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::messages::MessageType;

/// Current protocol schema version. Bump on incompatible changes; old agents
/// must receive an explicit protocol error, never silent misparse (WP-02).
pub const PROTOCOL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u32,
    pub minor: u32,
}

/// Control message envelope. Large payloads (media) MUST NOT travel inside
/// the envelope — they use pre-signed URLs / chunked transfer (TDD 12.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub message_id: String,
    pub correlation_id: String,
    pub sent_at: DateTime<Utc>,
    pub schema_version: u32,
    pub tenant_id: String,
    pub device_id: String,
    pub message_type: MessageType,
    /// Message payload (small JSON). Media references are keys/URLs, not bytes.
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn new(
        message_type: MessageType,
        tenant_id: impl Into<String>,
        device_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            message_id: Uuid::new_v4().to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            sent_at: Utc::now(),
            schema_version: PROTOCOL_SCHEMA_VERSION,
            tenant_id: tenant_id.into(),
            device_id: device_id.into(),
            message_type,
            payload,
        }
    }

    /// Round-trip through JSON — golden-fixture style check shared with Kotlin side.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_json_roundtrip() {
        let env = Envelope::new(
            MessageType::DeviceHeartbeat,
            "agency-1",
            "device-1",
            serde_json::json!({"last_seen": "2026-11-20T00:00:00Z"}),
        );
        let raw = env.to_json().expect("to_json");
        let back = Envelope::from_json(&raw).expect("from_json");
        assert_eq!(back.message_id, env.message_id);
        assert_eq!(back.schema_version, PROTOCOL_SCHEMA_VERSION);
        assert_eq!(back.message_type, MessageType::DeviceHeartbeat);
    }
}
