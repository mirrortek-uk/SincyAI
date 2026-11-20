//! Digital Employee domain (SINCY-ARCH-DE-001).
//!
//! The Digital Employee owns business intent and decision-making; it consumes
//! capability APIs (World Intelligence, Content Factory / LLM, Publishing
//! Execution) through port traits and never touches platform APIs or devices
//! directly (DE §3 boundary rule, §25.9 tool rule).

pub mod agents;
pub mod cycle;
pub mod domain;
pub mod lifecycle;
pub mod memory;
pub mod skills;

pub use agents::{AgentKind, AgentRequest, AgentResult};
pub use cycle::{CycleDeps, CycleOutcome, CycleStep, CycleStore, run_cycle};
pub use domain::{Employee, EmployeeStatus, Goal, Identity, Mission, Strategy, StrategyStatus};
pub use lifecycle::{LifecycleError, transition};
pub use memory::{MemoryItem, MemoryType};
pub use skills::{Skill, SkillDomain, agent_skill_domains, pick_content_skill, skills_for_agent};
