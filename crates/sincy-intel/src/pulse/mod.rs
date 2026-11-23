//! Sincy Pulse — Extensible Intelligence Provider Plugin Framework (Phase 1+2).
//!
//! 第三方站点只是 Intelligence Infrastructure: Provider → Adapter →
//! 统一实体(带 observation) → 跨源聚合 → Sincy Pulse 产品输出。
//! 红线: legal_status 非 public/commercial_ok 的 provider 不调度;
//! 采集只走公开只读接口, 不绕认证/验证码/限流。

pub mod entity;
pub mod manifest;
pub mod provider;

pub use entity::{AggregateMetrics, IntelligenceEntity, Observation, aggregate, classify_trend};
pub use manifest::{Acquisition, AcquisitionType, Dataset, LegalStatus, ProviderManifest};
pub use provider::{
    BuiltinFeedProvider, FeedFetcher, IntelligenceProvider, ProviderHealth, WebFetcher,
    WebScrapeProvider, extract_html_items, extract_json_items,
};
