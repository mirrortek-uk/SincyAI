//! UI drift detection (FR-022, TDD 9.3): an adapter run drifts when the App
//! version is out of range, anchor match rate falls below threshold, or steps
//! time out repeatedly. Drift blocks execution and marks the adapter degraded.

use crate::schema::Anchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftReason {
    VersionOutOfRange,
    AnchorMatchLow,
    StepTimeout,
}

#[derive(Debug, Clone)]
pub struct DriftVerdict {
    pub drifted: bool,
    pub reason: Option<DriftReason>,
    pub anchor_match_rate: f64,
}

pub const ANCHOR_MATCH_THRESHOLD: f64 = 0.6;

/// Fraction of the screen's anchors that matched the live snapshot.
pub fn anchor_match_rate(
    anchors: &[Anchor],
    snapshot: &dyn crate::interpreter::ScreenSnapshot,
) -> f64 {
    if anchors.is_empty() {
        return 0.0;
    }
    let matched = anchors
        .iter()
        .filter(|a| anchor_matches(a, snapshot))
        .count();
    matched as f64 / anchors.len() as f64
}

pub fn anchor_matches(a: &Anchor, snapshot: &dyn crate::interpreter::ScreenSnapshot) -> bool {
    if let Some(id) = &a.id
        && !snapshot.has_node_with_id(id)
    {
        return false;
    }
    if let Some(text) = &a.text
        && !snapshot.has_text(text)
    {
        return false;
    }
    if let Some(any) = &a.text_any
        && !any.iter().any(|t| snapshot.has_text(t))
    {
        return false;
    }
    true
}

pub fn judge(anchor_rate: f64, consecutive_timeouts: u32, version_in_range: bool) -> DriftVerdict {
    if !version_in_range {
        return DriftVerdict {
            drifted: true,
            reason: Some(DriftReason::VersionOutOfRange),
            anchor_match_rate: anchor_rate,
        };
    }
    if anchor_rate < ANCHOR_MATCH_THRESHOLD {
        return DriftVerdict {
            drifted: true,
            reason: Some(DriftReason::AnchorMatchLow),
            anchor_match_rate: anchor_rate,
        };
    }
    if consecutive_timeouts >= 3 {
        return DriftVerdict {
            drifted: true,
            reason: Some(DriftReason::StepTimeout),
            anchor_match_rate: anchor_rate,
        };
    }
    DriftVerdict {
        drifted: false,
        reason: None,
        anchor_match_rate: anchor_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::MockSnapshot;

    #[test]
    fn drift_judgements() {
        assert!(!judge(0.9, 0, true).drifted);
        assert_eq!(
            judge(0.9, 0, false).reason,
            Some(DriftReason::VersionOutOfRange)
        );
        assert_eq!(
            judge(0.3, 0, true).reason,
            Some(DriftReason::AnchorMatchLow)
        );
        assert_eq!(judge(0.9, 3, true).reason, Some(DriftReason::StepTimeout));
    }

    #[test]
    fn anchor_match_rate_computation() {
        let anchors = vec![
            Anchor {
                id: Some("a".into()),
                text: None,
                text_any: None,
            },
            Anchor {
                id: None,
                text: Some("首页".into()),
                text_any: None,
            },
            Anchor {
                id: None,
                text: None,
                text_any: Some(vec!["发布".into()]),
            },
        ];
        let snap = MockSnapshot::new(vec!["a".into()], vec!["首页".into(), "发布".into()]);
        assert!((anchor_match_rate(&anchors, &snap) - 1.0).abs() < 1e-9);
        let snap2 = MockSnapshot::new(vec![], vec!["别的".into()]);
        assert_eq!(anchor_match_rate(&anchors, &snap2), 0.0);
    }
}
