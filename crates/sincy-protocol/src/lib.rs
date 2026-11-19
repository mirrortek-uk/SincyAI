//! sincy-protocol: control-plane message model for the Agent WebSocket gateway
//! (TDD 12.1/12.3, WP-02).
//!
//! Transport is WSS + Protobuf in production; this crate defines the semantic
//! model first (serde), and `proto/` holds the .proto sources used to generate
//! the Rust/Kotlin bindings. schema_version mismatches must be rejected explicitly.

pub mod envelope;
pub mod messages;

/// prost-generated bindings from proto/*.proto (WP-02).
/// Package `sincy.protocol.v1` generates one module per file.
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/sincy.protocol.v1.rs"));
}

pub use envelope::{Envelope, PROTOCOL_SCHEMA_VERSION, SchemaVersion};
pub use messages::{DeviceMessage, MessageType, PublishingMessage, TransferMessage};

/// Errors for protocol handling.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unsupported schema_version {0}: this build supports {PROTOCOL_SCHEMA_VERSION}")]
    UnsupportedSchema(u32),
    #[error("invalid envelope: {0}")]
    InvalidEnvelope(String),
    #[error("unknown message type: {0}")]
    UnknownMessageType(String),
}

/// Validate an envelope's schema_version against this build.
pub fn check_schema(env: &Envelope) -> Result<(), ProtocolError> {
    if env.schema_version != PROTOCOL_SCHEMA_VERSION {
        return Err(ProtocolError::UnsupportedSchema(env.schema_version));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_mismatch_is_rejected() {
        let env = Envelope::new(
            MessageType::DeviceHeartbeat,
            "tenant-1",
            "device-1",
            serde_json::json!({}),
        );
        let mut bad = env.clone();
        bad.schema_version = 999;
        assert!(matches!(
            check_schema(&bad),
            Err(ProtocolError::UnsupportedSchema(999))
        ));
        assert!(check_schema(&env).is_ok());
    }

    /// WP-02 golden fixtures: every fixture must parse into the serde model,
    /// round-trip through the prost-generated protobuf Envelope, and keep its
    /// message_type stable across both representations (shared with Kotlin side).
    #[test]
    fn golden_fixtures_roundtrip_through_prost() {
        use prost::Message as _;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("fixtures dir")
            .map(|e| e.expect("entry").path())
            .collect();
        entries.sort();
        assert!(entries.len() >= 3, "expected golden fixtures");
        for path in entries {
            let raw = std::fs::read_to_string(&path).expect("read fixture");
            let env = Envelope::from_json(&raw).expect("serde parse");
            check_schema(&env).expect("schema");

            // prost roundtrip
            let pb = proto::Envelope {
                message_id: env.message_id.clone(),
                correlation_id: env.correlation_id.clone(),
                sent_at: Some(prost_types::Timestamp {
                    seconds: env.sent_at.timestamp(),
                    nanos: env.sent_at.timestamp_subsec_nanos() as i32,
                }),
                schema_version: env.schema_version,
                tenant_id: env.tenant_id.clone(),
                device_id: env.device_id.clone(),
                message_type: env.message_type.as_str().to_string(),
                payload: serde_json::to_vec(&env.payload).expect("payload json"),
            };
            let mut buf = Vec::new();
            pb.encode(&mut buf).expect("encode");
            let back = proto::Envelope::decode(&buf[..]).expect("decode");
            assert_eq!(back.message_id, env.message_id, "{}", path.display());
            assert_eq!(back.message_type, env.message_type.as_str());
            let payload_back: serde_json::Value =
                serde_json::from_slice(&back.payload).expect("payload decode");
            assert_eq!(payload_back, env.payload, "{}", path.display());
        }
    }
}
