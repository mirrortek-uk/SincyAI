//! sincy-device (WP-07): device identity, registration challenge-response,
//! session tokens, capability/app-version reporting, revocation.
//!
//! Acceptance (WP-07): 伪造 device_id 无法认证; 撤销后连接失败; 能力与 App
//! 版本更新有审计. Secrets are stored as hashes only.

pub mod manager;

pub use manager::{DeviceError, DeviceManager, DeviceRecord, RegistrationChallenge, SessionToken};
