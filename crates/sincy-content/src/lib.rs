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
pub mod voice;

pub use ai_label::{AiLabel, PlatformLabelRequirement};
pub use factory::{
    Brief, CommandProvider, Learning, LocalStubProvider, MasterAsset, MediaJob, MediaOutput,
    MediaProvider, MetricEvent, PlatformAdapter, PlatformSpec, SimpleAdapter, StrategyPlan,
    TextGenerator, TriggerSource, adapt_master, create_master, create_master_with_voice,
    default_adapters, learn, plan_strategy, plan_strategy_with_learnings,
};
pub use gate::{GateVerdict, QualityGate, QualityIssue};
pub use package::{MediaAsset, PackageEditor, PackageError, PublishingPackage};
pub use voice::{
    VoiceExemplar, VoiceProfile, VoiceSource, VoiceViolation, banned_move_keywords,
    compile_voice_prompt, deep_merge, distill_prompt, effective_voice, merge_opt, parse_distill,
    voice_violations,
};
