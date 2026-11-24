//! AdbIo — real on-screen touch driven from a computer via adb.
//!
//! Works against an Android emulator OR a USB-connected phone (adb authorized):
//! - perception: `adb shell uiautomator dump` → pull the XML → node list with
//!   resource-id / text / bounds (same info the phone's accessibility tree gives)
//! - actions: `adb shell input tap x y` / `input swipe ...` / `input text ...` /
//!   `input keyevent BACK` — these are INJECTED input events, i.e. real simulated
//!   touch on the device screen.
//!
//! This is how the computer side operates X without a physical finger: the
//! adapter interpreter (same signed code as on-device) decides WHAT to tap;
//! AdbIo only translates to coordinates.

use std::path::PathBuf;
use std::process::Command;

use sincy_adapter::humanize::{HumanProfile, Rng, plan_swipe, plan_tap};
use sincy_adapter::interpreter::{Action, PlatformIo, ScreenSnapshot};
use sincy_adapter::schema::{Anchor, FlowStep};

#[derive(Debug, Clone)]
pub struct UiNode {
    pub id: Option<String>,
    pub text: Option<String>,
    pub desc: Option<String>,
    /// (left, top, right, bottom)
    pub bounds: Option<(i32, i32, i32, i32)>,
    pub clickable: bool,
    pub focused: bool,
    pub editable: bool,
}

impl UiNode {
    fn center(&self) -> Option<(i32, i32)> {
        self.bounds.map(|(l, t, r, b)| ((l + r) / 2, (t + b) / 2))
    }
    fn haystack(&self) -> String {
        format!(
            "{} {} {}",
            self.id.clone().unwrap_or_default(),
            self.text.clone().unwrap_or_default(),
            self.desc.clone().unwrap_or_default()
        )
    }
}

fn anchor_matches(n: &UiNode, a: &Anchor) -> bool {
    if let Some(id) = &a.id
        && let Some(nid) = &n.id
        && (nid == id || nid.ends_with(&format!("/{id}")) || nid.ends_with(&format!(":{id}")))
    {
        return true;
    }
    if let Some(t) = &a.text
        && (n.text.as_deref() == Some(t.as_str()) || n.desc.as_deref() == Some(t.as_str()))
    {
        return true;
    }
    if let Some(any) = &a.text_any {
        let hay = n.haystack();
        if any.iter().any(|t| hay.contains(t.as_str())) {
            return true;
        }
    }
    if let Some(near) = &a.near_text_any {
        let hay = n.haystack();
        if near.iter().any(|t| hay.contains(t.as_str())) {
            return true;
        }
    }
    // no constraints at all -> match any clickable node
    if a.id.is_none() && a.text.is_none() && a.text_any.is_none() && a.near_text_any.is_none() {
        return n.clickable;
    }
    false
}

/// Snapshot parsed from `uiautomator dump` XML.
pub struct AdbSnapshot {
    pub nodes: Vec<UiNode>,
}

impl ScreenSnapshot for AdbSnapshot {
    fn has_node_with_id(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| {
            n.id.as_ref()
                .map(|nid| {
                    nid == id
                        || nid.ends_with(&format!("/{id}"))
                        || nid.ends_with(&format!(":{id}"))
                })
                .unwrap_or(false)
        })
    }
    fn has_text(&self, text: &str) -> bool {
        self.nodes.iter().any(|n| {
            n.text
                .as_deref()
                .map(|t| t.trim() == text.trim())
                .unwrap_or(false)
                || n.desc
                    .as_deref()
                    .map(|t| t.trim() == text.trim())
                    .unwrap_or(false)
        })
    }
}

/// Parse `uiautomator dump` XML without an XML crate: scan `<node ...>` tags.
pub fn parse_uiautomator_xml(xml: &str) -> Vec<UiNode> {
    let mut nodes = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find("<node ") {
        let tag_end = rest[pos..].find('>').map(|e| pos + e).unwrap_or(rest.len());
        let tag = &rest[pos..tag_end];
        let get = |attr: &str| -> Option<String> {
            let key = format!("{attr}=\"");
            let i = tag.find(&key)? + key.len();
            let j = tag[i..].find('"')? + i;
            Some(tag[i..j].to_string())
        };
        let bounds = get("bounds").and_then(|b| {
            // format: "[l,t][r,b]"
            let nums: Vec<i32> = b
                .split(['[', ']', ','])
                .filter(|s| !s.trim().is_empty())
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            if nums.len() == 4 {
                Some((nums[0], nums[1], nums[2], nums[3]))
            } else {
                None
            }
        });
        nodes.push(UiNode {
            id: get("resource-id").filter(|s| !s.is_empty()),
            text: get("text").filter(|s| !s.is_empty()),
            desc: get("content-desc").filter(|s| !s.is_empty()),
            bounds,
            clickable: get("clickable").as_deref() == Some("true"),
            focused: get("focused").as_deref() == Some("true"),
            editable: get("class")
                .map(|c| c.ends_with("EditText"))
                .unwrap_or(false),
        });
        rest = &rest[tag_end..];
    }
    nodes
}

/// Escape a string for `adb shell input text` (space -> %s, specials -> %XX).
pub fn adb_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%s"),
            '%' => out.push_str("%25"),
            c if c.is_ascii()
                && (c.is_alphanumeric() || "-_.,:;!?/\\@#+*()[]{}'\"".contains(c)) =>
            {
                out.push(c)
            }
            c => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// PlatformIo over adb: real injected touch events on an emulator or USB phone.
/// All gestures go through the shared humanizer (sincy_adapter::humanize) so
/// taps/swipes look human: off-center jittered taps, bowed bezier swipe paths,
/// ease-in-out segment timing, random pre/post delays.
pub struct AdbIo {
    pub device: String,
    pub workdir: PathBuf,
    pub performed: Vec<String>,
    pub last_snapshot: Vec<UiNode>,
    pub profile: HumanProfile,
    pub rng: Rng,
    screen_size: Option<(i32, i32)>,
}

impl AdbIo {
    pub fn new(device: String) -> Self {
        Self::with_rng(device, Rng::from_time())
    }

    /// Deterministic-seed constructor (sim `--seed`, reproducible runs).
    pub fn with_seed(device: String, seed: u64) -> Self {
        Self::with_rng(device, Rng::new(seed))
    }

    pub fn with_rng(device: String, rng: Rng) -> Self {
        let workdir = std::env::temp_dir().join("sincy-adb-io");
        std::fs::create_dir_all(&workdir).ok();
        Self {
            device,
            workdir,
            performed: vec![],
            last_snapshot: vec![],
            profile: HumanProfile::default(),
            rng,
            screen_size: None,
        }
    }

    fn sleep_ms(ms: u64) {
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    /// Physical screen size via `wm size` (cached).
    fn screen_size(&mut self) -> (i32, i32) {
        if let Some(s) = self.screen_size {
            return s;
        }
        let out = self.shell("wm size");
        let txt = String::from_utf8_lossy(&out.stdout);
        let size = txt
            .split_whitespace()
            .last()
            .and_then(|s| {
                s.split('x').next().and_then(|w| w.parse().ok()).map(|w| {
                    let h = s
                        .split('x')
                        .nth(1)
                        .and_then(|h| h.parse().ok())
                        .unwrap_or(2400);
                    (w, h)
                })
            })
            .unwrap_or((1080, 2400));
        self.screen_size = Some(size);
        size
    }

    fn adb(&self, args: &[&str]) -> std::process::Output {
        let mut full = vec!["-s", &self.device];
        full.extend_from_slice(args);
        Command::new("adb")
            .args(&full)
            .output()
            .unwrap_or_else(|e| {
                panic!("failed to run adb (is it on PATH?): {e}");
            })
    }

    fn shell(&self, cmd: &str) -> std::process::Output {
        self.adb(&["shell", cmd])
    }

    fn fetch_snapshot(&mut self) -> AdbSnapshot {
        let remote = "/sdcard/sincy_ui.xml";
        let local = self.workdir.join("ui.xml");
        // uiautomator dump returns non-zero sometimes; ignore status
        let _ = self.shell(&format!("uiautomator dump {remote}"));
        let pull = self.adb(&["pull", remote, local.to_str().unwrap_or("ui.xml")]);
        if !pull.status.success() {
            eprintln!("adb pull failed: {}", String::from_utf8_lossy(&pull.stderr));
        }
        let xml = std::fs::read_to_string(&local).unwrap_or_default();
        let nodes = parse_uiautomator_xml(&xml);
        self.last_snapshot = nodes.clone();
        AdbSnapshot { nodes }
    }

    fn find_node(&self, anchor: &Anchor) -> Option<&UiNode> {
        self.last_snapshot
            .iter()
            .find(|n| anchor_matches(n, anchor))
            .or_else(|| {
                self.last_snapshot
                    .iter()
                    .find(|n| n.clickable && anchor_matches(n, anchor))
            })
    }
}

impl AdbIo {
    /// read 原语: 找 near_text_any 标签节点, 在其后的兄弟节点里取第一个带数字的文本;
    /// 若锚点自身文本带数字则直接返回。只读, 不点击。
    fn read_near(&mut self, target: &Anchor) -> Result<Option<String>, String> {
        self.fetch_snapshot();
        let nodes = &self.last_snapshot;
        let is_count = |t: &str| t.chars().any(|c| c.is_ascii_digit());
        if let Some(near) = &target.near_text_any {
            if let Some(pos) = nodes.iter().position(|n| {
                let hay = n.haystack();
                near.iter().any(|t| hay.contains(t.as_str()))
            }) {
                // 标签节点自身带数字 (如 "1,234 关注者") 也接受
                if let Some(t) = nodes[pos].text.as_deref().filter(|t| is_count(t)) {
                    return Ok(Some(t.to_string()));
                }
                // 否则扫描其后若干兄弟节点 (uiautomator 平铺文档序)
                for n in nodes.iter().skip(pos + 1).take(6) {
                    if let Some(t) = n.text.as_deref().filter(|t| is_count(t)) {
                        return Ok(Some(t.to_string()));
                    }
                }
                return Ok(None);
            }
            return Ok(None);
        }
        if let Some(t) = target.text.as_deref().or(target
            .text_any
            .as_ref()
            .and_then(|v| v.first().map(|s| s.as_str())))
            && let Some(n) = nodes.iter().find(|n| n.haystack().contains(t))
            && let Some(v) = n.text.as_deref().filter(|t| is_count(t))
        {
            return Ok(Some(v.to_string()));
        }
        Ok(None)
    }
}

impl PlatformIo for AdbIo {
    fn read(&mut self, target: &Anchor) -> Result<Option<String>, String> {
        self.read_near(target)
    }

    fn perform_system_op(
        &mut self,
        op: &sincy_adapter::interpreter::SystemOp,
    ) -> Result<String, String> {
        use sincy_adapter::interpreter::SystemOp;
        match op {
            // 本地模拟: 下载由宿主机 curl 完成并校验 sha256
            SystemOp::DownloadFile { url, sha256 } => {
                let out = std::env::temp_dir().join("sincy_setup_download.apk");
                let st = std::process::Command::new("curl")
                    .args(["-sSL", url, "-o"])
                    .arg(&out)
                    .status()
                    .map_err(|e| e.to_string())?;
                if !st.success() {
                    return Err(format!("curl failed: {st}"));
                }
                let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
                use sha2::Digest;
                let got = format!("{:x}", sha2::Sha256::digest(&bytes));
                if !got.eq_ignore_ascii_case(sha256) {
                    return Err(format!("sha256 mismatch: {got}"));
                }
                Ok(format!("downloaded {} bytes", bytes.len()))
            }
            SystemOp::InstallApk => {
                let apk = std::env::temp_dir().join("sincy_setup_download.apk");
                let st = std::process::Command::new("adb")
                    .args(["install", "-r"])
                    .arg(&apk)
                    .status()
                    .map_err(|e| e.to_string())?;
                if st.success() {
                    Ok("installed".into())
                } else {
                    Err("adb install failed".into())
                }
            }
            SystemOp::WaitInstalled {
                package,
                timeout_ms,
            } => {
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_millis(*timeout_ms);
                while std::time::Instant::now() < deadline {
                    let out = std::process::Command::new("adb")
                        .args(["shell", "pm", "list", "packages", package])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if String::from_utf8_lossy(&out.stdout).contains(package) {
                        return Ok("installed".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                }
                Err("timeout waiting for install".into())
            }
            SystemOp::VerifySigner { .. } => Ok("ok (sim: signer not checked)".into()),
            SystemOp::LaunchApp { package } => {
                let out = std::process::Command::new("adb")
                    .args([
                        "shell",
                        "monkey",
                        "-p",
                        package,
                        "-c",
                        "android.intent.category.LAUNCHER",
                        "1",
                    ])
                    .output()
                    .map_err(|e| e.to_string())?;
                if out.status.success() {
                    Ok("launched".into())
                } else {
                    Err("launch failed".into())
                }
            }
            SystemOp::WaitMs { ms } => {
                std::thread::sleep(std::time::Duration::from_millis(*ms));
                Ok("ok".into())
            }
        }
    }

    fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
        Box::new(self.fetch_snapshot())
    }

    fn perform(&mut self, step: &FlowStep) -> Result<(), String> {
        // refresh node positions first (screen may have changed)
        self.fetch_snapshot();
        let Some(action) = step.to_action(0) else {
            return Err("step has no performable action".into());
        };
        let desc = format!("{action:?}");
        match &action {
            Action::Tap(anchor) => {
                let node = self
                    .find_node(anchor)
                    .ok_or_else(|| format!("no node for anchor {anchor:?}"))?;
                let bounds = node
                    .bounds
                    .ok_or_else(|| format!("node {:?} has no bounds", node.id))?;
                let plan = plan_tap(bounds, &self.profile, &mut self.rng);
                Self::sleep_ms(plan.pre_ms);
                let o = self.shell(&format!("input tap {} {}", plan.x, plan.y));
                if !o.status.success() {
                    return Err(format!(
                        "input tap failed: {}",
                        String::from_utf8_lossy(&o.stderr)
                    ));
                }
                Self::sleep_ms(plan.post_ms);
            }
            Action::SetText { target, value } => {
                let node = self
                    .find_node(target)
                    .ok_or_else(|| format!("no node for set_text target {target:?}"))?;
                if let Some((x, y)) = node.center() {
                    // human-like focus tap: jittered, with pre/post delays
                    let plan = plan_tap(
                        node.bounds.unwrap_or((x - 50, y - 20, x + 50, y + 20)),
                        &self.profile,
                        &mut self.rng,
                    );
                    Self::sleep_ms(plan.pre_ms);
                    let _ = self.shell(&format!("input tap {} {}", plan.x, plan.y));
                    Self::sleep_ms(plan.post_ms);
                }
                // type in chunks with human gaps instead of one instant dump
                for chunk in value.split_inclusive(' ') {
                    let piece = chunk.trim_end_matches(' ');
                    if !piece.is_empty() {
                        let o = self.shell(&format!("input text {}", adb_escape(piece)));
                        if !o.status.success() {
                            return Err(format!(
                                "input text failed: {}",
                                String::from_utf8_lossy(&o.stderr)
                            ));
                        }
                    }
                    if chunk.ends_with(' ') {
                        let _ = self.shell(&format!("input text {}", adb_escape(" ")));
                    }
                    Self::sleep_ms(self.rng.range(60.0, 220.0) as u64);
                }
            }
            Action::Back => {
                Self::sleep_ms(self.rng.range(120.0, 450.0) as u64);
                let _ = self.shell("input keyevent KEYCODE_BACK");
            }
            Action::Scroll => {
                let (w, h) = self.screen_size();
                let plan = plan_swipe(
                    w / 2,
                    (h as f64 * 0.72) as i32,
                    w / 2 + self.rng.int(-30, 30),
                    (h as f64 * 0.28) as i32,
                    &self.profile,
                    &mut self.rng,
                );
                // adb `input swipe` only takes endpoints, so a curved human path
                // is approximated as chained short swipes; the APK backend runs
                // the same plan as ONE continuous multi-stroke gesture.
                for seg in &plan.segments {
                    let o = self.shell(&format!(
                        "input swipe {} {} {} {} {}",
                        seg.x1, seg.y1, seg.x2, seg.y2, seg.ms
                    ));
                    if !o.status.success() {
                        return Err(format!(
                            "input swipe failed: {}",
                            String::from_utf8_lossy(&o.stderr)
                        ));
                    }
                }
                Self::sleep_ms(self.rng.range(250.0, 700.0) as u64);
            }
            // 纯感知动作: read 不点击任何东西 (值由 PlatformIo::read 取), open_self_profile 走资料页入口
            Action::Read { .. } => {}
            Action::OpenSelfProfile => {
                let node = self
                    .last_snapshot
                    .iter()
                    .find(|n| {
                        n.clickable
                            && (n
                                .id
                                .as_deref()
                                .map(|i| i.contains("profile"))
                                .unwrap_or(false)
                                || ["Profile", "个人主页", "我"]
                                    .iter()
                                    .any(|t| n.haystack().contains(t)))
                    })
                    .ok_or_else(|| "no profile entry found".to_string())?;
                let bounds = node
                    .bounds
                    .ok_or_else(|| "profile node has no bounds".to_string())?;
                let plan = plan_tap(bounds, &self.profile, &mut self.rng);
                Self::sleep_ms(plan.pre_ms);
                let _ = self.shell(&format!("input tap {} {}", plan.x, plan.y));
                Self::sleep_ms(plan.post_ms);
            }
            Action::PickMedia { .. } | Action::CloseDialog => {
                Self::sleep_ms(self.rng.range(120.0, 450.0) as u64);
                let _ = self.shell("input keyevent KEYCODE_BACK");
            }
            Action::TapAt { x, y } => {
                let plan = plan_tap(
                    (*x - 8, *y - 8, *x + 8, *y + 8),
                    &self.profile,
                    &mut self.rng,
                );
                Self::sleep_ms(plan.pre_ms);
                let o = self.shell(&format!("input tap {} {}", plan.x, plan.y));
                if !o.status.success() {
                    return Err(format!(
                        "input tap failed: {}",
                        String::from_utf8_lossy(&o.stderr)
                    ));
                }
                Self::sleep_ms(plan.post_ms);
            }
            Action::SwipePath { x1, y1, x2, y2, ms } => {
                let mut plan = plan_swipe(*x1, *y1, *x2, *y2, &self.profile, &mut self.rng);
                let total: u64 = plan.segments.iter().map(|s| s.ms).sum();
                if total > 0 && *ms > 0 {
                    let scale = *ms as f64 / total as f64;
                    for s in plan.segments.iter_mut() {
                        s.ms = ((s.ms as f64) * scale).max(16.0) as u64;
                    }
                }
                for seg in &plan.segments {
                    let o = self.shell(&format!(
                        "input swipe {} {} {} {} {}",
                        seg.x1, seg.y1, seg.x2, seg.y2, seg.ms
                    ));
                    if !o.status.success() {
                        return Err(format!(
                            "input swipe failed: {}",
                            String::from_utf8_lossy(&o.stderr)
                        ));
                    }
                }
                Self::sleep_ms(self.rng.range(250.0, 700.0) as u64);
            }
            Action::Wait { ms } => {
                Self::sleep_ms(*ms);
            }
        }
        self.performed.push(desc);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uiautomator_xml() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy><node resource-id="com.twitter.android:id/composer" text="" content-desc="Compose" bounds="[0,100][1080,300]" clickable="true" focused="false" class="android.widget.Button"/><node resource-id="com.twitter.android:id/tweet_text" text="" bounds="[10,400][1070,600]" clickable="false" focused="true" class="android.widget.EditText"/></hierarchy>"#;
        let nodes = parse_uiautomator_xml(xml);
        assert_eq!(nodes.len(), 2);
        assert_eq!(
            nodes[0].id.as_deref(),
            Some("com.twitter.android:id/composer")
        );
        assert_eq!(nodes[0].bounds, Some((0, 100, 1080, 300)));
        assert!(nodes[0].clickable);
        assert!(nodes[1].editable);
        assert_eq!(nodes[0].center(), Some((540, 200)));
    }

    #[test]
    fn anchor_matching_by_suffix() {
        let n = UiNode {
            id: Some("com.twitter.android:id/post".into()),
            text: Some("Post".into()),
            desc: None,
            bounds: Some((0, 0, 10, 10)),
            clickable: true,
            focused: false,
            editable: false,
        };
        assert!(anchor_matches(
            &n,
            &Anchor {
                id: Some("post".into()),
                text: None,
                text_any: None,
                near_text_any: None,
            }
        ));
        assert!(anchor_matches(
            &n,
            &Anchor {
                id: None,
                text: Some("Post".into()),
                text_any: None,
                near_text_any: None,
            }
        ));
        assert!(anchor_matches(
            &n,
            &Anchor {
                id: None,
                text: None,
                text_any: Some(vec!["Po".into()]),
                near_text_any: None,
            }
        ));
        assert!(!anchor_matches(
            &n,
            &Anchor {
                id: Some("other".into()),
                text: None,
                text_any: None,
                near_text_any: None,
            }
        ));
    }

    #[test]
    fn adb_escape_rules() {
        assert_eq!(adb_escape("hello world"), "hello%sworld");
        assert_eq!(adb_escape("a b&c%"), "a%sb%26c%25");
        assert_eq!(adb_escape("你好"), "%E4%BD%A0%E5%A5%BD");
    }
}
