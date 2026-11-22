//! Agent control-channel client: WSS to the cloud gateway + HTTP task-detail
//! fetch, then execute the assigned task with a device PlatformIo.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use sincy_adapter::interpreter::{ExecEvent, PlatformIo};
use sincy_protocol::{Envelope, MessageType};
use thiserror::Error;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::execute::{AdapterExecution, TaskOutcome, execute_signed_adapter_ctx};

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("ws error: {0}")]
    Ws(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("execute error: {0}")]
    Execute(String),
    #[error("timeout waiting for task assign")]
    AssignTimeout,
}

/// Task detail as served by `GET /api/v1/agent/tasks/{id}` (device token auth).
#[derive(Debug, Clone, Deserialize)]
pub struct TaskBrief {
    pub task_id: String,
    pub platform: String,
    pub action: String,
    pub package_text: String,
    pub app_package: String,
    pub adapter_id: String,
    pub adapter_version: i32,
    pub adapter_yaml: String,
    pub signature: String,
    /// free-form task context (e.g. agentic goal) from device_tasks.context
    #[serde(default)]
    pub task_context: serde_json::Value,
}

pub struct AgentClient {
    /// Full WS url, e.g. ws://127.0.0.1:18080/api/v1/agent/connect?token=...
    pub ws_url: String,
    /// HTTP base, e.g. http://127.0.0.1:18080/api/v1
    pub http_base: String,
    pub token: String,
    pub device_id: String,
    pub agency_id: String,
    pub signing_key: Vec<u8>,
}

impl AgentClient {
    fn env(&self, mt: MessageType, payload: serde_json::Value) -> Envelope {
        Envelope::new(mt, self.agency_id.clone(), self.device_id.clone(), payload)
    }

    /// Fetch the task brief (package text + signed adapter) over authenticated HTTP.
    pub async fn fetch_task(&self, task_id: &str) -> Result<TaskBrief, AgentError> {
        let url = format!(
            "{}/agent/tasks/{}?token={}",
            self.http_base, task_id, self.token
        );
        let resp = reqwest::Client::new()
            .get(&url)
            .bearer_auth(&self.token) // auth middleware requires the Bearer header
            .send()
            .await
            .map_err(|e| AgentError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::Http(format!("task fetch {status}: {body}")));
        }
        resp.json::<TaskBrief>()
            .await
            .map_err(|e| AgentError::Http(e.to_string()))
    }

    /// Connect, heartbeat, wait for one device.task.assign, fetch its detail,
    /// execute it with `io_factory`, and stream ack/evidence/result envelopes.
    /// Returns the execution result. This is the whole device-side loop.
    pub async fn run_next_task<IOF>(
        &self,
        io_factory: IOF,
        app_version: &str,
        assign_timeout: std::time::Duration,
    ) -> Result<AdapterExecution, AgentError>
    where
        IOF: FnOnce() -> Box<dyn PlatformIo + Send>,
    {
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&self.ws_url)
            .await
            .map_err(|e| AgentError::Ws(e.to_string()))?;

        // heartbeat so the cloud marks the device active
        let hb = self.env(MessageType::DeviceHeartbeat, json!({"status": "online"}));
        ws.send(WsMessage::Text(
            hb.to_json()
                .map_err(|e| AgentError::Protocol(e.to_string()))?
                .into(),
        ))
        .await
        .map_err(|e| AgentError::Ws(e.to_string()))?;

        // wait for an assign
        let task_id = loop {
            let msg = tokio::time::timeout(assign_timeout, ws.next())
                .await
                .map_err(|_| AgentError::AssignTimeout)?;
            let Some(Ok(WsMessage::Text(text))) = msg else {
                continue;
            };
            let env =
                Envelope::from_json(&text).map_err(|e| AgentError::Protocol(e.to_string()))?;
            if env.message_type == MessageType::DeviceTaskAssign {
                break env.payload["task_id"]
                    .as_str()
                    .ok_or_else(|| AgentError::Protocol("assign without task_id".into()))?
                    .to_string();
            }
        };

        let brief = self.fetch_task(&task_id).await?;

        // ack
        let ack = self.env(
            MessageType::DeviceTaskAck,
            json!({"task_id": task_id, "state": "EXECUTING"}),
        );
        ws.send(WsMessage::Text(
            ack.to_json()
                .map_err(|e| AgentError::Protocol(e.to_string()))?
                .into(),
        ))
        .await
        .map_err(|e| AgentError::Ws(e.to_string()))?;

        let mut io = io_factory();
        let ctx = if brief.task_context.is_null() {
            None
        } else {
            Some(brief.task_context.clone())
        };
        let result = execute_signed_adapter_ctx(
            &brief.adapter_yaml,
            &brief.signature,
            &self.signing_key,
            app_version,
            &mut *io,
            0,
            ctx,
        )
        .map_err(|e| AgentError::Execute(e.to_string()))?;

        // stream step evidence
        for ev in &result.events {
            let payload = match ev {
                ExecEvent::Step(s) => json!({
                    "task_id": task_id,
                    "step": s.step_index,
                    "screen": s.screen_id,
                    "side_effect": s.side_effect,
                    "evidence": s.evidence,
                }),
                other => json!({"task_id": task_id, "event": format!("{other:?}")}),
            };
            let env = self.env(MessageType::DeviceStepEvidence, payload);
            let _ = ws
                .send(WsMessage::Text(
                    env.to_json()
                        .map_err(|e| AgentError::Protocol(e.to_string()))?
                        .into(),
                ))
                .await;
        }

        // dedicated envelopes for challenge / drift (never auto-solved)
        match &result.outcome {
            TaskOutcome::WaitingUser(reason) => {
                let env = self.env(
                    MessageType::DeviceChallengeDetected,
                    json!({"task_id": task_id, "reason": reason}),
                );
                let _ = ws
                    .send(WsMessage::Text(
                        env.to_json()
                            .map_err(|e| AgentError::Protocol(e.to_string()))?
                            .into(),
                    ))
                    .await;
            }
            TaskOutcome::UiDrift(reason) => {
                let env = self.env(
                    MessageType::DeviceUiDriftDetected,
                    json!({"task_id": task_id, "platform": brief.platform, "action": brief.action, "reason": reason}),
                );
                let _ = ws
                    .send(WsMessage::Text(
                        env.to_json()
                            .map_err(|e| AgentError::Protocol(e.to_string()))?
                            .into(),
                    ))
                    .await;
            }
            _ => {}
        }

        // final result
        let res = self.env(
            MessageType::DeviceTaskResult,
            json!({
                "task_id": task_id,
                "outcome": result.outcome.as_protocol(),
                "adapter_id": result.adapter_id,
                "adapter_version": result.adapter_version,
                "side_effect_done": result.side_effect_done,
                "detail": match &result.outcome {
                    TaskOutcome::Success => "completed".to_string(),
                    other => format!("{other:?}"),
                },
            }),
        );
        ws.send(WsMessage::Text(
            res.to_json()
                .map_err(|e| AgentError::Protocol(e.to_string()))?
                .into(),
        ))
        .await
        .map_err(|e| AgentError::Ws(e.to_string()))?;

        // grace period: let the gateway apply the result before the socket
        // closes (exiting immediately can truncate the final frame)
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        Ok(result)
    }
}
