//! Operating-cycle e2e with in-memory store + mock ports: opportunity →
//! LLM draft → critic gate → package → publish → verify → memory/insight.

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use sincy_employee::agents::AgentKind;
use sincy_employee::cycle::*;
use sincy_employee::domain::*;
use sincy_employee::memory::MemoryItem;
use sincy_intel::adapter::EmployeeContext;
use sincy_intel::model::Opportunity;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
struct MemStore {
    cycles: Mutex<Vec<(Uuid, String)>>,
    tasks: Mutex<Vec<(Uuid, String, String)>>,
    memories: Mutex<Vec<MemoryItem>>,
    insights: Mutex<Vec<String>>,
    opp_status: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl CycleStore for MemStore {
    async fn begin_cycle(&self, _e: Uuid, _t: &str, idem: &str) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        self.cycles
            .lock()
            .unwrap()
            .push((id, format!("begin:{idem}")));
        Ok(id)
    }
    async fn finish_cycle(&self, id: Uuid, state: &str, _s: &Value) -> Result<(), String> {
        self.cycles.lock().unwrap().push((id, state.to_string()));
        Ok(())
    }
    async fn begin_task(
        &self,
        _c: Uuid,
        agent: AgentKind,
        s: CycleStep,
        idem: &str,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        self.tasks.lock().unwrap().push((
            id,
            format!("{}:{}", agent.as_str(), s.as_str()),
            idem.to_string(),
        ));
        Ok(id)
    }
    async fn finish_task(&self, id: Uuid, state: &str, _o: &Value) -> Result<(), String> {
        self.tasks
            .lock()
            .unwrap()
            .push((id, state.to_string(), "".into()));
        Ok(())
    }
    async fn add_memory(&self, item: &MemoryItem) -> Result<(), String> {
        self.memories.lock().unwrap().push(item.clone());
        Ok(())
    }
    async fn add_insight(
        &self,
        _e: Uuid,
        _c: Uuid,
        content: &str,
        _ev: &Value,
        _conf: f64,
    ) -> Result<(), String> {
        self.insights.lock().unwrap().push(content.to_string());
        Ok(())
    }
    async fn recent_topics(&self, _e: Uuid, _l: usize) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    async fn mark_opportunity(&self, topic: &str, status: &str) -> Result<(), String> {
        self.opp_status
            .lock()
            .unwrap()
            .push((topic.into(), status.into()));
        Ok(())
    }
}

struct MockIntel;
#[async_trait]
impl IntelPort for MockIntel {
    async fn opportunities(
        &self,
        _ctx: &EmployeeContext,
        _l: usize,
    ) -> Result<Vec<Opportunity>, String> {
        Ok(vec![
            Opportunity {
                topic: "ai agents".into(),
                score: 0.8,
                rationale: json!({}),
                evidence: vec!["e1".into()],
                status: "new".into(),
            },
            Opportunity {
                topic: "boring stuff".into(),
                score: 0.1,
                rationale: json!({}),
                evidence: vec![],
                status: "new".into(),
            },
        ])
    }
}

struct MockLlm;
#[async_trait]
impl LlmPort for MockLlm {
    async fn generate(&self, _p: &str, _m: u32) -> Result<String, String> {
        Ok("AI agents are reshaping dev workflows — here is what to watch.".into())
    }
}

struct MockStats;
#[async_trait]
impl sincy_employee::StatsPort for MockStats {
    async fn metrics(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"device_tasks_by_state": [], "mock": true}))
    }
}

struct MockPublish {
    auto: bool,
}
#[async_trait]
impl PublishPort for MockPublish {
    async fn save_package(
        &self,
        _e: &Employee,
        topic: &str,
        text: &str,
    ) -> Result<(Uuid, String), String> {
        let pid = Uuid::new_v4();
        let _ = (topic, text);
        Ok((
            pid,
            if self.auto {
                "approved".into()
            } else {
                "pending".into()
            },
        ))
    }
    async fn approve_package(&self, _p: Uuid) -> Result<(), String> {
        Ok(())
    }
    async fn publish(&self, _e: &Employee, package_id: Uuid) -> Result<PublishReceipt, String> {
        Ok(PublishReceipt {
            task_id: Uuid::new_v4(),
            route: "device".into(),
            status: "DEVICE_QUEUED".into(),
            package_id,
            approval_status: "approved".into(),
        })
    }
    async fn task_state(&self, _t: Uuid) -> Result<String, String> {
        Ok("QUEUED".into())
    }
}

fn employee(auto: bool) -> Employee {
    Employee {
        id: Uuid::new_v4(),
        agency_id: Uuid::new_v4(),
        name: "Nova".into(),
        role: "AI content operator".into(),
        status: EmployeeStatus::Running,
        config_version: 1,
        brand_id: None,
        channel: "x".into(),
        auto_publish: auto,
        created_at: Utc::now(),
    }
}

fn identity() -> Identity {
    Identity {
        version: 1,
        name: "Nova".into(),
        role: "AI content operator".into(),
        personality: json!({}),
        voice: "sharp, practical".into(),
        voice_profile: json!({}),

        focus: json!({"scope":"general"}),
        language: "zh-CN".into(),
        audience: vec!["developers".into()],
        expertise: vec!["ai".into()],
        values: vec!["accuracy first".into()],
        boundaries: vec!["no hype".into()],
    }
}

fn mission() -> Mission {
    Mission {
        statement: "build AI authority on X".into(),
        domain: "ai".into(),
        audience: vec!["developers".into()],
        value_proposition: "practical AI takes".into(),
        constraints: vec!["no unverified claims".into()],
        success_definition: "growing engaged audience".into(),
    }
}

fn strategy() -> Strategy {
    Strategy {
        pillars: vec!["ai agents".into()],
        ..Default::default()
    }
}

#[tokio::test]
async fn cycle_with_approval_gate_parks_package() {
    let store = MemStore::default();
    let deps = CycleDeps {
        store: Box::new(MemStore::default()),
        intel: Box::new(MockIntel),
        llm: Box::new(MockLlm),
        publish: Box::new(MockPublish { auto: false }),
        stats: Box::new(MockStats),
    };
    let _ = &store;
    let out = run_cycle(
        &deps,
        &employee(false),
        &identity(),
        &mission(),
        &strategy(),
        &[],
        "manual",
        "c1",
    )
    .await
    .unwrap();
    assert_eq!(out.state, "SUCCEEDED");
    assert_eq!(out.selected_opportunity.as_deref(), Some("ai agents"));
    assert!(out.package_id.is_some());
    assert!(out.task_id.is_none(), "pending package must not publish");
}

#[tokio::test]
async fn cycle_auto_publish_goes_to_device_route() {
    let deps = CycleDeps {
        store: Box::new(MemStore::default()),
        intel: Box::new(MockIntel),
        llm: Box::new(MockLlm),
        publish: Box::new(MockPublish { auto: true }),
        stats: Box::new(MockStats),
    };
    let out = run_cycle(
        &deps,
        &employee(true),
        &identity(),
        &mission(),
        &strategy(),
        &[],
        "scheduled",
        "c2",
    )
    .await
    .unwrap();
    let receipt = out.summary["receipt"].clone();
    assert_eq!(receipt["route"], "device");
    assert_eq!(out.task_state.as_deref(), Some("QUEUED"));
    assert!(out.task_id.is_some());
}

#[tokio::test]
async fn cycle_without_opportunity_is_noop_success() {
    struct NoIntel;
    #[async_trait]
    impl IntelPort for NoIntel {
        async fn opportunities(
            &self,
            _c: &EmployeeContext,
            _l: usize,
        ) -> Result<Vec<Opportunity>, String> {
            Ok(vec![])
        }
    }
    let deps = CycleDeps {
        store: Box::new(MemStore::default()),
        intel: Box::new(NoIntel),
        llm: Box::new(MockLlm),
        publish: Box::new(MockPublish { auto: true }),
        stats: Box::new(MockStats),
    };
    let out = run_cycle(
        &deps,
        &employee(true),
        &identity(),
        &mission(),
        &strategy(),
        &[],
        "scheduled",
        "c3",
    )
    .await
    .unwrap();
    assert!(out.selected_opportunity.is_none());
    assert!(out.package_id.is_none());
}

#[tokio::test]
async fn overlong_draft_on_x_becomes_thread() {
    // X 默认审核门: 280 加权 + 超限拆 thread, 不再打回
    struct LongLlm;
    #[async_trait]
    impl LlmPort for LongLlm {
        async fn generate(&self, _p: &str, _m: u32) -> Result<String, String> {
            Ok("a".repeat(600)) // over X 280 limit
        }
    }
    let deps = CycleDeps {
        store: Box::new(MemStore::default()),
        intel: Box::new(MockIntel),
        llm: Box::new(LongLlm),
        publish: Box::new(MockPublish { auto: true }),
        stats: Box::new(MockStats),
    };
    let out = run_cycle(
        &deps,
        &employee(true),
        &identity(),
        &mission(),
        &strategy(),
        &[],
        "manual",
        "c4",
    )
    .await
    .unwrap();
    assert!(
        out.package_id.is_some(),
        "overlong draft on x must be packaged as a thread, not rejected; summary={}",
        out.summary
    );
}

#[tokio::test]
async fn critic_rejects_overlong_draft_on_reject_platform() {
    // 非 X 渠道默认 reject: 超限仍然打回
    struct LongLlm;
    #[async_trait]
    impl LlmPort for LongLlm {
        async fn generate(&self, _p: &str, _m: u32) -> Result<String, String> {
            Ok("a".repeat(300))
        }
    }
    let mut e = employee(true);
    e.channel = "mastodon".into(); // default rule: 40000/reject... use small via rule? default 40000 won't trigger
    e.channel = "x-reject-test".into();
    // default_for 非 x 是 40000, 300 字不会触发; 这里直接用自定义 store 规则更准:
    struct RejectStore(MemStore);
    #[async_trait]
    impl CycleStore for RejectStore {
        async fn begin_cycle(&self, e: Uuid, t: &str, i: &str) -> Result<Uuid, String> {
            self.0.begin_cycle(e, t, i).await
        }
        async fn finish_cycle(&self, c: Uuid, s: &str, su: &Value) -> Result<(), String> {
            self.0.finish_cycle(c, s, su).await
        }
        async fn begin_task(
            &self,
            c: Uuid,
            a: AgentKind,
            s: CycleStep,
            i: &str,
        ) -> Result<Uuid, String> {
            self.0.begin_task(c, a, s, i).await
        }
        async fn finish_task(&self, t: Uuid, s: &str, o: &Value) -> Result<(), String> {
            self.0.finish_task(t, s, o).await
        }
        async fn add_memory(&self, i: &MemoryItem) -> Result<(), String> {
            self.0.add_memory(i).await
        }
        async fn add_insight(
            &self,
            e: Uuid,
            c: Uuid,
            ct: &str,
            ev: &Value,
            cf: f64,
        ) -> Result<(), String> {
            self.0.add_insight(e, c, ct, ev, cf).await
        }
        async fn recent_topics(&self, e: Uuid, l: usize) -> Result<Vec<String>, String> {
            self.0.recent_topics(e, l).await
        }
        async fn mark_opportunity(&self, t: &str, s: &str) -> Result<(), String> {
            self.0.mark_opportunity(t, s).await
        }
        async fn review_rule(&self, platform: &str) -> Result<PlatformReviewRule, String> {
            Ok(PlatformReviewRule {
                platform: platform.into(),
                weighted_limit: 100,
                on_over_limit: "reject".into(),
            })
        }
    }
    let deps = CycleDeps {
        store: Box::new(RejectStore(MemStore::default())),
        intel: Box::new(MockIntel),
        llm: Box::new(LongLlm),
        publish: Box::new(MockPublish { auto: true }),
        stats: Box::new(MockStats),
    };
    let out = run_cycle(
        &deps,
        &e,
        &identity(),
        &mission(),
        &strategy(),
        &[],
        "manual",
        "c5",
    )
    .await
    .unwrap();
    assert!(
        out.package_id.is_none(),
        "overlong draft on reject platform must not be packaged; summary={}",
        out.summary
    );
    assert_eq!(out.summary["reason"], "critic rejected draft");
}
