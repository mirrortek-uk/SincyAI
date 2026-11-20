//! Target App version range matching (FR-022): adapter declares e.g.
//! ">=8.10 <8.20"; devices outside the range refuse to execute (UI drift guard).

use regex::Regex;
use std::cmp::Ordering;

#[derive(Debug, thiserror::Error)]
pub enum VersionRangeError {
    #[error("invalid version range: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct VersionRange {
    raw: String,
    lower: Option<(Vec<u32>, bool)>, // (version, inclusive)
    upper: Option<(Vec<u32>, bool)>,
}

fn parse_version(s: &str) -> Result<Vec<u32>, VersionRangeError> {
    s.trim()
        .split('.')
        .map(|p| {
            p.parse::<u32>()
                .map_err(|_| VersionRangeError::Invalid(format!("bad version component {p:?}")))
        })
        .collect()
}

fn cmp_version(a: &[u32], b: &[u32]) -> Ordering {
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

impl VersionRange {
    pub fn parse(raw: &str) -> Result<Self, VersionRangeError> {
        let re = Regex::new(r"^(>=|>|<=|<)?\s*([0-9]+(\.[0-9]+)*)$")
            .map_err(|e| VersionRangeError::Invalid(e.to_string()))?;
        let mut lower = None;
        let mut upper = None;
        for part in raw.split_whitespace().collect::<Vec<_>>().chunks(2) {
            // grammar: sequence of "op version" tokens separated by spaces,
            // e.g. ">=8.10 <8.20" parses as two tokens
            let _ = part;
        }
        // simpler: scan tokens pairwise
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let tok = tokens[i];
            let (op, ver) = if let Some(v) = tok.strip_prefix(">=") {
                (">=", v)
            } else if let Some(v) = tok.strip_prefix("<=") {
                ("<=", v)
            } else if let Some(v) = tok.strip_prefix('>') {
                (">", v)
            } else if let Some(v) = tok.strip_prefix('<') {
                ("<", v)
            } else {
                (">=", tok) // bare version means >= that version
            };
            let v = parse_version(ver)?;
            match op {
                ">=" => lower = Some((v, true)),
                ">" => lower = Some((v, false)),
                "<=" => upper = Some((v, true)),
                "<" => upper = Some((v, false)),
                _ => unreachable!(),
            }
            i += 1;
        }
        if lower.is_none() && upper.is_none() {
            return Err(VersionRangeError::Invalid(raw.to_string()));
        }
        Ok(Self {
            raw: raw.to_string(),
            lower,
            upper,
        })
    }

    pub fn contains(&self, version: &str) -> Result<bool, VersionRangeError> {
        let v = parse_version(version)?;
        if let Some((lo, incl)) = &self.lower {
            let ord = cmp_version(&v, lo);
            let ok = if *incl {
                ord != Ordering::Less
            } else {
                ord == Ordering::Greater
            };
            if !ok {
                return Ok(false);
            }
        }
        if let Some((hi, incl)) = &self.upper {
            let ord = cmp_version(&v, hi);
            let ok = if *incl {
                ord != Ordering::Greater
            } else {
                ord == Ordering::Less
            };
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_matching() {
        let r = VersionRange::parse(">=8.10 <8.20").expect("parse");
        assert!(r.contains("8.10").unwrap());
        assert!(r.contains("8.15.3").unwrap());
        assert!(!r.contains("8.20").unwrap());
        assert!(!r.contains("8.09").unwrap());
        assert!(!r.contains("9.0").unwrap());
    }

    #[test]
    fn bare_version_means_at_least() {
        let r = VersionRange::parse("8.10").unwrap();
        assert!(r.contains("8.10").unwrap());
        assert!(r.contains("9.0").unwrap());
        assert!(!r.contains("8.9").unwrap());
    }
}
