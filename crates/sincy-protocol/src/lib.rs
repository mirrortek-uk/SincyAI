//! sincy-protocol: control-plane message model for the Agent WebSocket gateway
//! (TDD 12.1/12.3, WP-02).
//!
//! Transport is WSS + Protobuf in production; this crate defines the semantic
//! model first (serde), and `proto/` holds the .proto sources used to generate
//! the Rust/Kotlin bindings. schema_version mismatches must be rejected explicitly.

pub mod envelope;
pub mod messages;

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
}
