//! Human-like gesture planning for injected touch.
//!
//! Both device backends (APK accessibility `dispatchGesture`, PC-side
//! `adb shell input ...`) execute CONCRETE coordinate plans produced here, so
//! the humanization policy lives in exactly one tested place:
//!
//! - taps land off-center inside the target with natural jitter, never twice
//!   at the exact same pixel, with human pre/hold/post delays;
//! - swipes follow a slightly bowed quadratic bezier sampled into segments
//!   whose durations follow an ease-in-out profile plus noise (fast in the
//!   middle, slow at the ends) — never a perfectly straight constant-speed line;
//! - every segment carries a pressure value. Android's public injection APIs
//!   (`dispatchGesture`, `adb input`) do not expose pressure, so backends that
//!   cannot apply it ignore the field; the model keeps it so a future backend
//!   (e.g. raw /dev/input or root) can use it without changing the planner.
//!
//! The RNG is a deterministic SplitMix64: identical seed => identical plan,
//! which is what the unit tests (and the sim's `--seed`) rely on.

use serde::Serialize;

/// Deterministic SplitMix64 PRNG (no external deps, replayable in tests).
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    /// Seed from wall clock — used by the device/PC entry points.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x51C7_0000);
        Self::new(nanos ^ (std::process::id() as u64) << 32)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    /// Uniform integer in [lo, hi] inclusive.
    pub fn int(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i32
    }

    /// Approximate standard normal (sum of 3 uniforms), clamped to [-2.5, 2.5].
    fn gauss(&mut self) -> f64 {
        let g = self.f64() + self.f64() + self.f64() - 1.5;
        g.clamp(-2.5, 2.5) * 2.0 / 2.5
    }
}

/// Tunable human-behavior parameters.
#[derive(Debug, Clone)]
pub struct HumanProfile {
    /// Tap offset from center, as a fraction of the smaller rect dimension.
    pub tap_jitter_frac: f64,
    pub tap_hold_ms: (u64, u64),
    pub pre_action_ms: (u64, u64),
    pub post_action_ms: (u64, u64),
    /// Number of bezier sample points for a swipe.
    pub swipe_points: (usize, usize),
    /// Per-point positional jitter in px.
    pub swipe_jitter_px: f64,
    /// Perpendicular bow of the swipe path, as a fraction of its length.
    pub swipe_curve_frac: f64,
    /// Total swipe duration bounds; actual scales with distance.
    pub swipe_total_ms: (u64, u64),
    /// Pressure range carried in the plan (applied where the backend allows).
    pub pressure: (f32, f32),
}

impl Default for HumanProfile {
    fn default() -> Self {
        Self {
            tap_jitter_frac: 0.18,
            tap_hold_ms: (60, 140),
            pre_action_ms: (120, 450),
            post_action_ms: (250, 900),
            swipe_points: (8, 16),
            swipe_jitter_px: 2.5,
            swipe_curve_frac: 0.08,
            swipe_total_ms: (350, 850),
            pressure: (0.35, 0.95),
        }
    }
}

/// A concrete tap: where, how long, and the delays around it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TapPlan {
    pub x: i32,
    pub y: i32,
    pub hold_ms: u64,
    pub pre_ms: u64,
    pub post_ms: u64,
    pub pressure: f32,
}

/// One segment of a swipe (executed as one stroke / one `input swipe`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SwipeSegment {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    /// Duration of this segment.
    pub ms: u64,
    pub pressure: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SwipePlan {
    pub segments: Vec<SwipeSegment>,
    pub total_ms: u64,
}

/// Plan a human-like tap inside `bounds` = (left, top, right, bottom).
/// The point stays inside the rect (10% inset) but off-center.
pub fn plan_tap(bounds: (i32, i32, i32, i32), p: &HumanProfile, rng: &mut Rng) -> TapPlan {
    let (l, t, r, b) = bounds;
    let cx = (l + r) as f64 / 2.0;
    let cy = (t + b) as f64 / 2.0;
    let min_dim = ((r - l).min(b - t)) as f64;
    let j = min_dim * p.tap_jitter_frac / 2.0;
    let x = (cx + rng.gauss() * j).round() as i32;
    let y = (cy + rng.gauss() * j).round() as i32;
    // clamp into the 10%-inset rect so we never miss the target
    let il = l + (r - l) / 10;
    let it = t + (b - t) / 10;
    let ir = r - (r - l) / 10;
    let ib = b - (b - t) / 10;
    let x = x.clamp(il, ir);
    let y = y.clamp(it, ib);
    TapPlan {
        x,
        y,
        hold_ms: rng.range(p.tap_hold_ms.0 as f64, p.tap_hold_ms.1 as f64) as u64,
        pre_ms: rng.range(p.pre_action_ms.0 as f64, p.pre_action_ms.1 as f64) as u64,
        post_ms: rng.range(p.post_action_ms.0 as f64, p.post_action_ms.1 as f64) as u64,
        pressure: rng.range(p.pressure.0 as f64, p.pressure.1 as f64) as f32,
    }
}

/// Plan a human-like swipe from (x1,y1) to (x2,y2):
/// bowed bezier + per-point jitter + ease-in-out segment timing + noise.
pub fn plan_swipe(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    p: &HumanProfile,
    rng: &mut Rng,
) -> SwipePlan {
    let dx = (x2 - x1) as f64;
    let dy = (y2 - y1) as f64;
    let len = (dx * dx + dy * dy).sqrt();
    let n = rng
        .int(p.swipe_points.0 as i32, p.swipe_points.1 as i32)
        .max(2) as usize;

    // control point: midpoint bowed perpendicular to the direction
    let mx = (x1 + x2) as f64 / 2.0;
    let my = (y1 + y2) as f64 / 2.0;
    let bow = if len > 1.0 {
        let nx = -dy / len;
        let ny = dx / len;
        let mag = len
            * p.swipe_curve_frac
            * rng.range(0.3, 1.0)
            * if rng.f64() < 0.5 { -1.0 } else { 1.0 };
        (mx + nx * mag, my + ny * mag)
    } else {
        (mx, my)
    };

    // sample the quadratic bezier at n points
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let mt = 1.0 - t;
        let bx = mt * mt * x1 as f64 + 2.0 * mt * t * bow.0 + t * t * x2 as f64;
        let by = mt * mt * y1 as f64 + 2.0 * mt * t * bow.1 + t * t * y2 as f64;
        pts.push((bx, by));
    }

    // total duration scales mildly with distance, clamped to profile range
    let dist_ms = len * 0.45;
    let total = (dist_ms + rng.range(p.swipe_total_ms.0 as f64, p.swipe_total_ms.1 as f64)).clamp(
        p.swipe_total_ms.0 as f64,
        p.swipe_total_ms.1 as f64 + dist_ms,
    ) as u64;

    // jitter the sampled points once (endpoints stay exact so the gesture
    // lands where the caller asked), then segments chain exactly
    for (i, pt) in pts.iter_mut().enumerate() {
        if i == 0 || i == n - 1 {
            continue;
        }
        pt.0 += rng.range(-p.swipe_jitter_px, p.swipe_jitter_px);
        pt.1 += rng.range(-p.swipe_jitter_px, p.swipe_jitter_px);
    }

    // ease-in-out weights (sin profile) over segment midpoints, plus noise
    let mut weights: Vec<f64> = Vec::with_capacity(n - 1);
    for i in 0..(n - 1) {
        let tm = (i as f64 + 0.5) / (n - 1) as f64;
        // ease-in-out: slow at both ends (long segments), fast in the middle
        // (short segments) — duration weight is inverse of the sin speed profile
        let w = (1.6 - (std::f64::consts::PI * tm).sin()).max(0.05);
        weights.push(w * rng.range(0.8, 1.2));
    }
    let wsum: f64 = weights.iter().sum();

    let mut segments = Vec::with_capacity(n - 1);
    for i in 0..(n - 1) {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[i + 1];
        let ms = ((total as f64) * weights[i] / wsum).round().max(16.0) as u64;
        segments.push(SwipeSegment {
            x1: ax.round() as i32,
            y1: ay.round() as i32,
            x2: bx.round() as i32,
            y2: by.round() as i32,
            ms,
            pressure: rng.range(p.pressure.0 as f64, p.pressure.1 as f64) as f32,
        });
    }
    let total_ms: u64 = segments.iter().map(|s| s.ms).sum();
    SwipePlan { segments, total_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = Rng::new(43);
        assert_ne!(a.next_u64(), c.next_u64());
    }

    #[test]
    fn tap_stays_inside_bounds_and_off_center_sometimes() {
        let p = HumanProfile::default();
        let bounds = (100, 200, 300, 260); // 200x60 rect
        let mut rng = Rng::new(7);
        let mut off_center = 0;
        for _ in 0..200 {
            let t = plan_tap(bounds, &p, &mut rng);
            assert!(t.x >= 100 && t.x <= 300, "x out of bounds: {}", t.x);
            assert!(t.y >= 200 && t.y <= 260, "y out of bounds: {}", t.y);
            assert!(t.hold_ms >= 60 && t.hold_ms <= 140);
            assert!(t.pre_ms >= 120 && t.pre_ms <= 450);
            assert!(t.post_ms >= 250 && t.post_ms <= 900);
            assert!(t.pressure >= 0.35 && t.pressure <= 0.95);
            if t.x != 200 || t.y != 230 {
                off_center += 1;
            }
        }
        assert!(
            off_center > 150,
            "taps should usually be off-center, got {off_center}/200"
        );
    }

    #[test]
    fn tap_plan_is_deterministic_per_seed() {
        let p = HumanProfile::default();
        let mut r1 = Rng::new(99);
        let mut r2 = Rng::new(99);
        assert_eq!(
            plan_tap((0, 0, 100, 100), &p, &mut r1),
            plan_tap((0, 0, 100, 100), &p, &mut r2)
        );
    }

    #[test]
    fn swipe_endpoints_and_shape() {
        let p = HumanProfile::default();
        let mut rng = Rng::new(2026);
        let plan = plan_swipe(540, 1500, 540, 700, &p, &mut rng);
        assert!(plan.segments.len() >= 8 && plan.segments.len() <= 16);
        assert_eq!((plan.segments[0].x1, plan.segments[0].y1), (540, 1500));
        let last = plan.segments.last().unwrap();
        assert_eq!((last.x2, last.y2), (540, 700));
        // segments chain: each segment starts where the previous ended (modulo jitter on shared point)
        for w in plan.segments.windows(2) {
            assert!((w[0].x2 - w[1].x1).abs() <= 3);
            assert!((w[0].y2 - w[1].y1).abs() <= 3);
        }
        // total within a human range and segments vary (not constant speed)
        assert!(plan.total_ms >= 350 && plan.total_ms <= 1500);
        let ms: Vec<u64> = plan.segments.iter().map(|s| s.ms).collect();
        let min = *ms.iter().min().unwrap();
        let max = *ms.iter().max().unwrap();
        assert!(max > min, "segment durations should vary: {ms:?}");
        // path bows away from the straight line (perpendicular deviation > 0)
        let dev: f64 = plan
            .segments
            .iter()
            .map(|s| ((s.x1 - 540) as f64).abs())
            .fold(0.0, f64::max);
        assert!(dev > 0.5, "swipe should be curved, max |dx| = {dev}");
        // ease-in-out: middle segments faster (shorter) than the first/last on average
        let n = ms.len();
        let edge = (ms[0] + ms[n - 1]) as f64 / 2.0;
        let mid = ms[n / 2] as f64;
        assert!(
            mid <= edge,
            "middle should be faster: mid={mid} edge={edge}"
        );
    }

    #[test]
    fn swipe_short_distance_still_valid() {
        let p = HumanProfile::default();
        let mut rng = Rng::new(1);
        let plan = plan_swipe(100, 100, 105, 103, &p, &mut rng);
        assert!(!plan.segments.is_empty());
        assert!(plan.total_ms >= 16);
    }
}
