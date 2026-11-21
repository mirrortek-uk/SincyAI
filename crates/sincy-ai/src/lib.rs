//! sincy-ai (WP-14): model gateway abstraction, cost budget circuit breaker
//! (FR-026: 超阈值告警或熔断), and analytics feedback (FR-016: 平台指标回流到
//! 内容版本, 反馈策略层).
//!
//! Mock gateways are explicitly labeled and never presented as real providers.

pub mod analytics;
pub mod budget;
pub mod gateway;

pub use analytics::{Analytics, MetricImport};
pub use budget::{BudgetChecker, BudgetError, CostRecord};
pub use gateway::{
    GenerationRequest, GenerationResult, LlmGateway, ManualFileGateway, MockLlmGateway,
    OpenAiCompatibleGateway, gateway_from_env,
};
