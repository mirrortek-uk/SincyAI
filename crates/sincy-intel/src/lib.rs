//! World Intelligence Station core (Sincy World Intelligence Architecture Guide).
//!
//! Layers implemented here: source registry (L1), provider registry + router
//! (L3, scoring + fallback), unified intelligence model (L6 output schemas),
//! opportunity engine (L6), and the Digital Employee intelligence adapter
//! (L7, WI §20). Ingestion (L4/L5) lands normalized items via `ingest_item`.
//!
//! Red line (WI §29): connectors must not bypass platform authentication,
//! CAPTCHAs, or rate limits. This crate never talks to platforms directly —
//! it only routes *requests* to registered providers and normalizes results.

pub mod adapter;
pub mod model;
pub mod opportunity;
pub mod registry;
pub mod router;

pub use adapter::{EmployeeBrief, adapt_for_employee};
pub use model::{AccessMethod, IntelItem, IntelKind, Opportunity, SocialContent, Source, Trend};
pub use opportunity::{rank_opportunities, score_opportunity};
pub use registry::{Provider, ProviderRegistry, SourceRegistry};
pub use router::{RouteDecision, Router, RouterRequest, route};
