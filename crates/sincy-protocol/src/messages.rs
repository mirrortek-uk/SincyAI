//! Message catalog (TDD 12.3). One enum per direction keeps the gateway
//! routing explicit; payloads are documented per variant.

use serde::{Deserialize, Serialize};

/// All control message types exchanged over the Agent WSS gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    // device -> cloud
    DeviceRegister,
    DeviceHeartbeat,
    DeviceCapabilitiesUpdate,
    DeviceAppVersionsUpdate,
    DeviceTaskAck,
    DeviceTaskProgress,
    DeviceTaskResult,
    DeviceStepEvidence,
    DeviceUiDriftDetected,
    DeviceChallengeDetected,
    // cloud -> device
    DeviceTaskAssign,
    AdapterBundleUpdate,
    DeviceSessionRevoke,
    // transfer
    TransferRequest,
    TransferProgress,
    TransferComplete,
    // publishing execution
    PublishingExecutionRequest,
    PublishingExecutionResult,
}

/// Coarse message family used for routing/logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageFamily {
    Device,
    Adapter,
    Transfer,
    Publishing,
}

impl MessageType {
    pub fn family(&self) -> MessageFamily {
        use MessageType::*;
        match self {
            DeviceRegister
            | DeviceHeartbeat
            | DeviceCapabilitiesUpdate
            | DeviceAppVersionsUpdate
            | DeviceTaskAck
            | DeviceTaskProgress
            | DeviceTaskResult
            | DeviceStepEvidence
            | DeviceUiDriftDetected
            | DeviceChallengeDetected
            | DeviceTaskAssign
            | DeviceSessionRevoke => MessageFamily::Device,
            AdapterBundleUpdate => MessageFamily::Adapter,
            TransferRequest | TransferProgress | TransferComplete => MessageFamily::Transfer,
            PublishingExecutionRequest | PublishingExecutionResult => MessageFamily::Publishing,
        }
    }

    pub fn as_str(&self) -> &'static str {
        use MessageType::*;
        match self {
            DeviceRegister => "device.register",
            DeviceHeartbeat => "device.heartbeat",
            DeviceCapabilitiesUpdate => "device.capabilities.update",
            DeviceAppVersionsUpdate => "device.app_versions.update",
            DeviceTaskAssign => "device.task.assign",
            DeviceTaskAck => "device.task.ack",
            DeviceTaskProgress => "device.task.progress",
            DeviceTaskResult => "device.task.result",
            DeviceStepEvidence => "device.step.evidence",
            DeviceUiDriftDetected => "device.ui_drift.detected",
            DeviceChallengeDetected => "device.challenge.detected",
            AdapterBundleUpdate => "adapter.bundle.update",
            TransferRequest => "transfer.request",
            TransferProgress => "transfer.progress",
            TransferComplete => "transfer.complete",
            PublishingExecutionRequest => "publishing.execution.request",
            PublishingExecutionResult => "publishing.execution.result",
            DeviceSessionRevoke => "device.session.revoke",
        }
    }
}

/// Typed view helpers for device messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceMessage {
    pub message_type: MessageType,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferMessage {
    pub message_type: MessageType,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishingMessage {
    pub message_type: MessageType,
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matches_tdd_12_3() {
        let all = [
            MessageType::DeviceRegister,
            MessageType::DeviceHeartbeat,
            MessageType::DeviceCapabilitiesUpdate,
            MessageType::DeviceAppVersionsUpdate,
            MessageType::DeviceTaskAssign,
            MessageType::DeviceTaskAck,
            MessageType::DeviceTaskProgress,
            MessageType::DeviceTaskResult,
            MessageType::DeviceStepEvidence,
            MessageType::DeviceUiDriftDetected,
            MessageType::DeviceChallengeDetected,
            MessageType::AdapterBundleUpdate,
            MessageType::TransferRequest,
            MessageType::TransferProgress,
            MessageType::TransferComplete,
            MessageType::PublishingExecutionRequest,
            MessageType::PublishingExecutionResult,
            MessageType::DeviceSessionRevoke,
        ];
        assert_eq!(all.len(), 18);
        assert_eq!(
            MessageType::DeviceChallengeDetected.family(),
            MessageFamily::Device
        );
        assert_eq!(
            MessageType::TransferComplete.family(),
            MessageFamily::Transfer
        );
        assert_eq!(
            MessageType::PublishingExecutionRequest.family(),
            MessageFamily::Publishing
        );
    }
}
