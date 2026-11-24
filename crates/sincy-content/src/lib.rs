//! sincy-content (WP-04): content versions, assets/rights, AI labels, the
//! quality/policy gate, and immutable Publishing Packages (TDD 13).
//!
//! Immutability is enforced by the type system: `PublishingPackage` fields are
//! private with read-only accessors; edits produce a NEW version via
//! `PackageEditor`, never mutate an existing one (FR-009 / DoD: 包生成后不可原地篡改).

pub mod ai_label;
pub mod factory;
pub mod gate;
pub mod package;

pub use ai_label::{AiLabel, PlatformLabelRequirement};
pub use factory::{
    Brief, CommandProvider, Learning, LocalStubProvider, MasterAsset, MediaJob, MediaOutput,
    MediaProvider, MetricEvent, PlatformAdapter, PlatformSpec, SimpleAdapter, StrategyPlan,
    TextGenerator, TriggerSource, adapt_master, create_master, default_adapters, learn,
    plan_strategy,
};
pub use gate::{GateVerdict, QualityGate, QualityIssue};
pub use package::{MediaAsset, PackageEditor, PackageError, PublishingPackage};
