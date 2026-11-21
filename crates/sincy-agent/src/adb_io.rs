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
    // no constraints at all -> match any clickable node
    if a.id.is_none() && a.text.is_none() && a.text_any.is_none() {
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
pub struct AdbIo {
    pub device: String,
    pub workdir: PathBuf,
    pub performed: Vec<String>,
    pub last_snapshot: Vec<UiNode>,
}

impl AdbIo {
    pub fn new(device: String) -> Self {
        let workdir = std::env::temp_dir().join("sincy-adb-io");
        std::fs::create_dir_all(&workdir).ok();
        Self {
            device,
            workdir,
            performed: vec![],
            last_snapshot: vec![],
        }
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

impl PlatformIo for AdbIo {
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
                let (x, y) = node
                    .center()
                    .ok_or_else(|| format!("node {:?} has no bounds", node.id))?;
                let o = self.shell(&format!("input tap {x} {y}"));
                if !o.status.success() {
                    return Err(format!(
                        "input tap failed: {}",
                        String::from_utf8_lossy(&o.stderr)
                    ));
                }
            }
            Action::SetText { target, value } => {
                let node = self
                    .find_node(target)
                    .ok_or_else(|| format!("no node for set_text target {target:?}"))?;
                if let Some((x, y)) = node.center() {
                    let _ = self.shell(&format!("input tap {x} {y}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                let o = self.shell(&format!("input text {}", adb_escape(value)));
                if !o.status.success() {
                    return Err(format!(
                        "input text failed: {}",
                        String::from_utf8_lossy(&o.stderr)
                    ));
                }
            }
            Action::Back => {
                let _ = self.shell("input keyevent KEYCODE_BACK");
            }
            Action::Scroll => {
                let _ = self.shell("input swipe 540 1500 540 700 300");
            }
            Action::PickMedia { .. } | Action::CloseDialog => {
                let _ = self.shell("input keyevent KEYCODE_BACK");
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
                text_any: None
            }
        ));
        assert!(anchor_matches(
            &n,
            &Anchor {
                id: None,
                text: Some("Post".into()),
                text_any: None
            }
        ));
        assert!(anchor_matches(
            &n,
            &Anchor {
                id: None,
                text: None,
                text_any: Some(vec!["Po".into()])
            }
        ));
        assert!(!anchor_matches(
            &n,
            &Anchor {
                id: Some("other".into()),
                text: None,
                text_any: None
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
