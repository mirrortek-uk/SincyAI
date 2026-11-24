//! WP-13 acceptance: 示例 adapter 在模拟屏幕流上完整执行并留下证据;
//! 挑战界面必然停住转人工; App 版本越界阻断且零副作用; side_effect 后
//! 失败进入 VERIFICATION_PENDING; 签名篡改被拒; schema 校验生效.

use sincy_adapter::interpreter::{
    ExecStatus, Interpreter, MockSnapshot, PlatformIo, ScreenSnapshot, identify_screen,
};
use sincy_adapter::schema::{AdapterDefinition, FlowStep};
use sincy_adapter::sign::{definition_sha256, sign_definition, verify_definition};
use sincy_adapter::version_range::VersionRange;
use std::collections::BTreeMap;

fn sample_def() -> AdapterDefinition {
    let yaml = include_str!("../../../adapters/xiaohongshu.publish_video.yaml");
    AdapterDefinition::from_yaml(yaml).expect("sample adapter must validate")
}

/// A scripted platform: each snapshot() pops the next screen; perform() records actions.
struct ScriptedIo {
    screens: Vec<MockSnapshot>,
    performed: Vec<String>,
    fail_on_side_effect: bool,
}

impl PlatformIo for ScriptedIo {
    fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
        let s = if self.screens.is_empty() {
            MockSnapshot::new(vec![], vec![])
        } else {
            self.screens.remove(0)
        };
        Box::new(s)
    }
    fn perform(&mut self, step: &FlowStep) -> Result<(), String> {
        self.performed.push(format!("{:?}", step.action));
        if step.side_effect && self.fail_on_side_effect {
            return Err("post-publish network drop".into());
        }
        Ok(())
    }
}

fn home() -> MockSnapshot {
    MockSnapshot::new(
        vec![
            "com.xingin.xhs:id/home_tab".into(),
            "com.xingin.xhs:id/plus_button".into(),
        ],
        vec!["首页".into()],
    )
}
fn picker() -> MockSnapshot {
    MockSnapshot::new(vec![], vec!["选择视频".into()])
}
fn composer() -> MockSnapshot {
    MockSnapshot::new(
        vec![
            "com.xingin.xhs:id/title".into(),
            "com.xingin.xhs:id/body".into(),
        ],
        vec!["发布笔记".into(), "发布".into()],
    )
}
fn confirm() -> MockSnapshot {
    MockSnapshot::new(vec![], vec!["发布成功".into(), "完成".into()])
}
fn challenge() -> MockSnapshot {
    MockSnapshot::new(vec![], vec!["安全检测".into()])
}
fn unknown() -> MockSnapshot {
    MockSnapshot::new(vec!["zzz".into()], vec!["???".into()])
}

#[test]
fn happy_path_completes_with_evidence_and_side_effect_marked() {
    let def = sample_def();
    // flow: home, picker, composer(x2), composer(publish), confirm(verify)
    let io_screens = vec![
        home(),
        picker(),
        composer(),
        composer(),
        composer(),
        confirm(),
        confirm(),
    ];
    let mut io = ScriptedIo {
        screens: io_screens,
        performed: vec![],
        fail_on_side_effect: false,
    };
    let mut interp = Interpreter::new(&def, "8.15.0");
    let rt = interp.run(&mut io, 1);
    assert_eq!(rt.status, Some(ExecStatus::Completed));
    assert!(rt.side_effect_done, "publish tap is a side effect");
    assert!(
        io.performed.iter().any(|a| a.contains("Tap")),
        "tap actions performed"
    );
    // every step left evidence
    let steps = rt
        .events
        .iter()
        .filter(|e| matches!(e, sincy_adapter::interpreter::ExecEvent::Step(_)))
        .count();
    assert_eq!(steps, def.flow.len());
}

#[test]
fn challenge_screen_parks_task_for_human() {
    let def = sample_def();
    // home ok, then challenge appears instead of picker
    let mut io = ScriptedIo {
        screens: vec![home(), challenge()],
        performed: vec![],
        fail_on_side_effect: false,
    };
    let mut interp = Interpreter::new(&def, "8.15.0");
    let rt = interp.run(&mut io, 1);
    assert!(matches!(rt.status, Some(ExecStatus::WaitingUser(_))));
    assert!(!rt.side_effect_done, "no side effect before challenge");
    assert!(rt.events.iter().any(|e| matches!(
        e,
        sincy_adapter::interpreter::ExecEvent::ChallengeDetected { .. }
    )));
}

#[test]
fn unknown_screens_exhaust_safe_recovery_then_drift_blocks() {
    let def = sample_def();
    // home ok, then 5 unknown screens (recovery budget is 3)
    let mut screens = vec![home()];
    screens.extend((0..5).map(|_| unknown()));
    let mut io = ScriptedIo {
        screens,
        performed: vec![],
        fail_on_side_effect: false,
    };
    let mut interp = Interpreter::new(&def, "8.15.0");
    let rt = interp.run(&mut io, 1);
    // anchors fully mismatch after safe recoveries exhausted => UI drift, blocked
    assert!(
        matches!(rt.status, Some(ExecStatus::UiDrift(_))),
        "got {:?}",
        rt.status
    );
    // step 0 performed its normal (non-side-effect) tap; all recovery actions were Back only
    assert!(
        io.performed[1..].iter().all(|a| a.contains("Back")),
        "{:?}",
        io.performed
    );
    assert!(!rt.side_effect_done);
}

#[test]
fn app_version_out_of_range_blocks_with_zero_side_effects() {
    let def = sample_def();
    let mut io = ScriptedIo {
        screens: vec![home()],
        performed: vec![],
        fail_on_side_effect: false,
    };
    let mut interp = Interpreter::new(&def, "9.0.0"); // out of >=8.10 <8.20
    let rt = interp.run(&mut io, 1);
    assert!(matches!(rt.status, Some(ExecStatus::UiDrift(_))));
    assert!(
        io.performed.is_empty(),
        "version mismatch must not tap anything"
    );
    assert!(!rt.side_effect_done);
}

#[test]
fn side_effect_step_failure_is_verification_pending() {
    let def = sample_def();
    let io_screens = vec![home(), picker(), composer(), composer(), composer()];
    let mut io = ScriptedIo {
        screens: io_screens,
        performed: vec![],
        fail_on_side_effect: true,
    };
    let mut interp = Interpreter::new(&def, "8.15.0");
    let rt = interp.run(&mut io, 1);
    assert!(matches!(
        rt.status,
        Some(ExecStatus::VerificationPending(_))
    ));
    assert!(
        rt.side_effect_done,
        "side effect boundary crossed: no auto retry/degrade"
    );
}

#[test]
fn sample_adapter_signature_roundtrip_and_tamper_rejected() {
    let yaml = include_str!("../../../adapters/xiaohongshu.publish_video.yaml");
    let def = AdapterDefinition::from_yaml(yaml).expect("valid");
    let canonical = serde_json::to_string(&def).expect("canonical json");
    let key = b"sincy-platform-signing-key";
    let sig = sign_definition(key, &canonical);
    assert!(verify_definition(key, &canonical, &sig).is_ok());
    let tampered = canonical.replace("发布笔记", "篡改");
    assert!(verify_definition(key, &tampered, &sig).is_err());
    assert_eq!(definition_sha256(&canonical).len(), 64);
}

#[test]
fn schema_rejects_adapter_without_challenge_screen() {
    let bad = r#"
adapter: demo.no_challenge
version: 1
app: { package: com.demo, version_range: ">=1.0" }
screens:
  home: { anchors: [ { text: "首页" } ] }
flow:
  - { expect: home, action: back }
"#;
    assert!(AdapterDefinition::from_yaml(bad).is_err());
}

#[test]
fn version_range_gates_execution() {
    let def = sample_def();
    let range = VersionRange::parse(&def.app.version_range).unwrap();
    assert!(range.contains("8.10").unwrap());
    assert!(!range.contains("8.20").unwrap());
    let m = identify_screen(
        &MockSnapshot::new(vec![], vec!["验证".into()]),
        &def.screens,
        def.challenge_screen(),
    );
    assert_eq!(m.screen_id, "challenge");
}

#[test]
fn screen_identification_prefers_challenge() {
    let def = sample_def();
    let mut screens: BTreeMap<String, sincy_adapter::schema::ScreenDef> = def.screens.clone();
    let _ = &mut screens;
    // a screen that matches both home and challenge anchors must resolve to challenge
    let both = MockSnapshot::new(
        vec!["com.xingin.xhs:id/home_tab".into()],
        vec!["首页".into(), "安全检测".into()],
    );
    let m = identify_screen(&both, &def.screens, def.challenge_screen());
    assert_eq!(m.screen_id, "challenge");
}

#[test]
fn x_publish_post_adapter_validates_and_signs() {
    // X 走设备路线 (PRD 5.3 / HID 设计书, 不付费 API):
    // 签名 adapter 必须通过 schema 校验、可签名验签、且声明挑战界面。
    let yaml = include_str!("../../../adapters/x.publish_post.yaml");
    let def = AdapterDefinition::from_yaml(yaml).expect("x adapter must validate");
    assert_eq!(def.adapter, "x.publish_post");
    assert_eq!(def.app.package, "com.twitter.android");
    assert!(def.challenge_screen().is_some());
    let has_side_effect = def.flow.iter().any(|s| s.side_effect);
    assert!(has_side_effect, "posting must be marked side_effect");
    let has_verify = def.flow.iter().any(|s| s.verify);
    assert!(has_verify, "flow must end in a verify step");
    let canonical = serde_json::to_string(&def).expect("canonical json");
    let key = b"sincy-platform-signing-key";
    let sig = sign_definition(key, &canonical);
    assert!(verify_definition(key, &canonical, &sig).is_ok());
    let tampered = canonical.replace("com.twitter.android", "com.evil.app");
    assert!(verify_definition(key, &tampered, &sig).is_err());
    assert_eq!(definition_sha256(&canonical).len(), 64);
}

#[test]
fn x_publish_post_flow_executes_on_scripted_screens() {
    let yaml = include_str!("../../../adapters/x.publish_post.yaml");
    let def = AdapterDefinition::from_yaml(yaml).unwrap();
    let io = ScriptedIo {
        screens: vec![
            MockSnapshot::new(
                vec!["com.twitter.android:id/bottom_bar".into()],
                vec!["Home".into()],
            ),
            MockSnapshot::new(
                vec!["com.twitter.android:id/tweet_text".into()],
                vec!["What's happening?".into()],
            ),
            MockSnapshot::new(
                vec!["com.twitter.android:id/tweet_text".into()],
                vec!["What's happening?".into()],
            ),
            MockSnapshot::new(vec![], vec!["Your post was sent".into()]),
            MockSnapshot::new(vec![], vec!["Your post was sent".into()]),
        ],
        performed: vec![],
        fail_on_side_effect: false,
    };
    let mut io = io;
    let mut interp = Interpreter::new(&def, "10.5.0");
    let rt = interp.run(&mut io, 0);
    assert_eq!(rt.status, Some(ExecStatus::Completed));
    assert!(rt.side_effect_done, "Post tap is a side effect");
}

/// read/capture 原语: 只读步骤把 io.read 的值按 capture key 收进 rt.captured,
/// 且不产生 side_effect; io 不支持 read 时 fail-closed。
#[test]
fn read_capture_collects_values() {
    use sincy_adapter::schema::Anchor;
    let yaml = r#"
adapter: x.read_followers
version: 1
app: { package: com.twitter.android, version_range: ">=10.0" }
screens:
  profile:
    anchors:
      - { text_any: ["Followers"] }
  challenge:
    anchors:
      - { text_any: ["人机验证", "Verify you are human"] }
flow:
  - { expect: profile, action: read, target: { near_text_any: ["Followers"] }, capture: "followers" }
"#;
    let def = AdapterDefinition::from_yaml(yaml).expect("read adapter must validate");

    struct ReadIo {
        screens: Vec<MockSnapshot>,
    }
    impl PlatformIo for ReadIo {
        fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
            let s = if self.screens.is_empty() {
                MockSnapshot::new(vec![], vec![])
            } else {
                self.screens.remove(0)
            };
            Box::new(s)
        }
        fn perform(&mut self, _step: &FlowStep) -> Result<(), String> {
            Ok(())
        }
        fn read(&mut self, target: &Anchor) -> Result<Option<String>, String> {
            assert!(target.near_text_any.is_some());
            Ok(Some("1,234".into()))
        }
    }
    let mut io = ReadIo {
        screens: vec![MockSnapshot::new(vec![], vec!["Followers".into()])],
    };
    let mut interp = Interpreter::new(&def, "10.5.0");
    let rt = interp.run(&mut io, 0);
    assert_eq!(rt.status, Some(ExecStatus::Completed));
    assert_eq!(
        rt.captured.get("followers").map(|s| s.as_str()),
        Some("1,234")
    );
    assert!(!rt.side_effect_done);

    // 不支持 read 的 io → fail-closed
    struct NoReadIo {
        screens: Vec<MockSnapshot>,
    }
    impl PlatformIo for NoReadIo {
        fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
            let s = if self.screens.is_empty() {
                MockSnapshot::new(vec![], vec![])
            } else {
                self.screens.remove(0)
            };
            Box::new(s)
        }
        fn perform(&mut self, _step: &FlowStep) -> Result<(), String> {
            Ok(())
        }
    }
    let mut io2 = NoReadIo {
        screens: vec![MockSnapshot::new(vec![], vec!["Followers".into()])],
    };
    let mut interp2 = Interpreter::new(&def, "10.5.0");
    let rt2 = interp2.run(&mut io2, 0);
    assert!(matches!(rt2.status, Some(ExecStatus::Failed(_))));
    assert!(rt2.captured.is_empty());
}
