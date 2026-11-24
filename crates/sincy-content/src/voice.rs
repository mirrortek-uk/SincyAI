//! Voice Profile(语气档案) — 数字员工的结构化「文字习惯 / 语气」规范。
//!
//! 设计见 设计书/Sincy-Voice-Profile.md:
//!   两级存储: 基线 employee_identity_versions.identity->'voice_profile'
//!             ⊕ 覆盖 employee_accounts.voice_profile (migration 0047)
//!   有效语气 = merge_opt(base, override)(覆盖为 null 的键保留基线)
//!
//! 三用:
//!   1. 风格参数 (register…structure_habits) 编译成紧凑指令块, 注入工序2/工序3与员工 prompt —— 软约束
//!   2. exemplars 做 few-shot 锁手感 —— 软约束
//!   3. avoid_words / banned_moves 交给审查门做硬约束 —— 违反直接 fail
//!
//! 边界: 克隆的是风格不是身份/原文; 产物必须过 originality/copyright 门。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---------- 通用 JSON 合并 (kpi.rs 复用同一份) ----------

/// deep merge: over 的对象键递归合并, 其余类型直接覆盖。
pub fn deep_merge(base: &Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out = b.clone();
            for (k, v) in o {
                let merged = match out.get(k) {
                    Some(old) => deep_merge(old, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => over.clone(),
    }
}

/// deep_merge 但 over 里任何层级的 Null 都保留 base (空 overrides 不擦掉基线/模板)。
pub fn merge_opt(base: &Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out = b.clone();
            for (k, v) in o {
                if v.is_null() {
                    continue; // 显式 null = 不覆盖, 保留 base
                }
                let merged = match out.get(k) {
                    Some(old) => merge_opt(old, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => {
            if over.is_null() {
                base.clone()
            } else {
                over.clone()
            }
        }
    }
}

/// 有效语气 = merge_opt(员工基线, 账号覆盖)。
pub fn effective_voice(base: &Value, account_override: &Value) -> Value {
    merge_opt(base, account_override)
}

// ---------- Voice Profile schema ----------

/// 结构化语气档案 (§2 schema)。全部字段可选, 存储为 JSONB。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceProfile {
    /// 基调: casual / 犀利 / 权威 / 温暖 / 毒舌 …
    pub register: Option<String>,
    /// 0 随意 .. 1 正式
    pub formality: Option<f64>,
    /// first_person / brand_we
    pub person: Option<String>,
    /// none / sparing / liberal
    pub emoji: Option<String>,
    pub hashtags: Option<HashtagStyle>,
    /// 大小写习惯 (lowercase_casual / standard …)
    pub capitalization: Option<String>,
    /// punchy 短促金句 / long_form / mixed
    pub rhythm: Option<String>,
    pub sentence_len: Option<String>,
    /// 口头禅 / 爱用词
    pub signature_phrases: Vec<String>,
    /// 禁用词 / 雷区 (硬约束)
    pub avoid_words: Vec<String>,
    pub humor: Option<String>,
    pub slang: Option<String>,
    /// 结构套路: hook_first / one_idea_per_post / thread_for_depth …
    pub structure_habits: Vec<String>,
    /// 立场 / 价值观口吻
    pub pov_values: Option<String>,
    /// 禁止的写法 (硬约束, 见 BANNED_MOVE_KEYWORDS)
    pub banned_moves: Vec<String>,
    /// few-shot 范例 (最强杠杆): 3-8 条「就该这样写」
    pub exemplars: Vec<VoiceExemplar>,
    /// 克隆来源 + 权重 (多账号混合蒸馏)
    pub source_accounts: Vec<VoiceSource>,
    pub distilled_at: Option<String>,
    pub distill_model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HashtagStyle {
    pub style: Option<String>,
    pub max: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceExemplar {
    pub text: String,
    pub source: Option<String>,
    pub why: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSource {
    pub handle: String,
    pub weight: Option<f64>,
}

impl VoiceProfile {
    pub fn from_value(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

// ---------- 编译成 prompt 指令块 (软约束, 注入生成) ----------

/// 把语气档案编译成一段紧凑风格指令块 + 少量 exemplars, 前置到生成 prompt。
/// 空档案返回空串 (调用方退回旧的 `Voice: {voice}` 单字段)。
pub fn compile_voice_prompt(profile: &Value, max_exemplars: usize) -> String {
    let p = VoiceProfile::from_value(profile);
    if p.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = vec![
        "VOICE PROFILE — match this voice exactly (style only, never copy exemplar text verbatim):"
            .into(),
    ];
    let mut push = |k: &str, v: String| lines.push(format!("- {k}: {v}"));
    if let Some(v) = &p.register {
        push("register", v.clone());
    }
    if let Some(v) = p.formality {
        push("formality", format!("{v} (0 casual .. 1 formal)"));
    }
    if let Some(v) = &p.person {
        push("person", v.clone());
    }
    if let Some(v) = &p.emoji {
        push("emoji", v.clone());
    }
    if let Some(h) = &p.hashtags {
        let style = h.style.clone().unwrap_or_else(|| "minimal".into());
        let max = h.max.unwrap_or(2);
        push("hashtags", format!("{style}, max {max}"));
    }
    if let Some(v) = &p.capitalization {
        push("capitalization", v.clone());
    }
    if let Some(v) = &p.rhythm {
        push("rhythm", v.clone());
    }
    if let Some(v) = &p.sentence_len {
        push("sentence_len", v.clone());
    }
    if let Some(v) = &p.humor {
        push("humor", v.clone());
    }
    if let Some(v) = &p.slang {
        push("slang", v.clone());
    }
    if let Some(v) = &p.pov_values {
        push("pov/values", v.clone());
    }
    if !p.signature_phrases.is_empty() {
        push("signature_phrases", p.signature_phrases.join(" | "));
    }
    if !p.structure_habits.is_empty() {
        push("structure_habits", p.structure_habits.join(", "));
    }
    if !p.avoid_words.is_empty() {
        lines.push(format!(
            "- HARD RULE avoid_words (never use): {}",
            p.avoid_words.join(", ")
        ));
    }
    if !p.banned_moves.is_empty() {
        lines.push(format!(
            "- HARD RULE banned_moves (never do): {}",
            p.banned_moves.join(", ")
        ));
    }
    let ex: Vec<&VoiceExemplar> = p.exemplars.iter().take(max_exemplars.max(1)).collect();
    if !ex.is_empty() {
        lines.push("Style exemplars (match the feel, write your own original text):".into());
        for (i, e) in ex.iter().enumerate() {
            let why = e
                .why
                .as_deref()
                .map(|w| format!("  // {w}"))
                .unwrap_or_default();
            lines.push(format!("{}. {}{}", i + 1, e.text.trim(), why));
        }
    }
    lines.join("\n")
}

// ---------- 硬约束检查 (审查门) ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceViolation {
    /// voice_avoid_word / voice_banned_move
    pub code: String,
    pub message: String,
}

/// banned_moves 的确定性关键词映射 (LLM 审查兜底语义型判定)。
pub fn banned_move_keywords(m: &str) -> &'static [&'static str] {
    match m {
        "engagement_bait" => &[
            "你同意吗",
            "评论区见",
            "留言告诉我",
            "说出你的看法",
            "comment below",
            "drop a",
            "agree?",
        ],
        "hard_sell" => &[
            "限时优惠",
            "立即购买",
            "点击链接",
            "扫码购买",
            "私信我",
            "dm me",
            "折扣码",
            "点击下方链接",
            "lowest price",
        ],
        "fear_mongering" => &["不转发就会", "不看后悔", "再不收藏就没了"],
        "impersonation" => &[],
        _ => &[],
    }
}

/// 硬约束: 命中 avoid_words / banned_moves 关键词 → 违规列表 (大小写不敏感)。
pub fn voice_violations(profile: &Value, text: &str) -> Vec<VoiceViolation> {
    let p = VoiceProfile::from_value(profile);
    let mut out = Vec::new();
    let lower = text.to_lowercase();
    for w in &p.avoid_words {
        if !w.is_empty() && lower.contains(&w.to_lowercase()) {
            out.push(VoiceViolation {
                code: "voice_avoid_word".into(),
                message: format!("命中语气禁用词: {w}"),
            });
        }
    }
    for m in &p.banned_moves {
        for kw in banned_move_keywords(m) {
            if lower.contains(&kw.to_lowercase()) {
                out.push(VoiceViolation {
                    code: "voice_banned_move".into(),
                    message: format!("命中禁写手法 {m}: {kw}"),
                });
            }
        }
    }
    out
}

// ---------- 蒸馏 (采样文本 → LLM → 结构化档案) ----------

/// 蒸馏 prompt: 把语料压成 §2 schema 的 JSON。
pub fn distill_prompt(samples: &[String], source_handles: &[String]) -> String {
    let corpus = samples
        .iter()
        .enumerate()
        .map(|(i, s)| format!("[{}] {}", i + 1, s.trim()))
        .collect::<Vec<_>>()
        .join("\n");
    let handles = if source_handles.is_empty() {
        "unknown".to_string()
    } else {
        source_handles.join(", ")
    };
    format!(
        "You are a writing-style analyst. Distill the following public posts into a structured VOICE PROFILE JSON.\n\
         Source handles: {handles}\n\
         Analyze: register (tone), formality (0 casual .. 1 formal), person, emoji usage, hashtag style, capitalization habit, rhythm, sentence length, signature phrases, words to avoid, humor, slang, structure habits (e.g. hook_first, one_idea_per_post), pov/values, banned moves (engagement_bait / hard_sell / fear_mongering), and pick 3-8 exemplars (text + why).\n\
         Output ONLY a JSON object with keys: register, formality, person, emoji, hashtags{{style,max}}, capitalization, rhythm, sentence_len, signature_phrases[], avoid_words[], humor, slang, structure_habits[], pov_values, banned_moves[], exemplars[{{text,source,why}}], source_accounts[{{handle,weight}}].\n\
         Style only — do not reproduce long verbatim passages in exemplars (max ~25 words each).\n\
         POSTS:\n{corpus}"
    )
}

/// 解析蒸馏结果: 取最后一个 JSON object, 校验为对象并补溯源字段。
pub fn parse_distill(raw: &str, model: &str) -> Result<Value, String> {
    // 扫描每个 '{' 位点, 用 stream 解析器取第一个 JSON 值; 接受第一个解析成功的 object
    let mut v: Option<Value> = None;
    for (i, ch) in raw.char_indices() {
        if ch != '{' {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&raw[i..]).into_iter::<Value>();
        if let Some(Ok(cand)) = stream.next()
            && cand.is_object()
        {
            v = Some(cand);
            break;
        }
    }
    let mut v = v.ok_or("distill: no JSON object in output")?;
    if !v.is_object() {
        return Err("distill: output is not a JSON object".into());
    }
    let obj = v.as_object_mut().unwrap();
    obj.insert(
        "distilled_at".into(),
        json!(chrono::Utc::now().to_rfc3339()),
    );
    obj.insert("distill_model".into(), json!(model));
    // 规范化 exemplars 上限, 防止 prompt 注入撑爆
    if let Some(ex) = obj.get_mut("exemplars").and_then(|e| e.as_array_mut()) {
        ex.truncate(8);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_opt_null_keeps_base() {
        let base = json!({"register": "witty", "emoji": "sparing"});
        let over = json!({"register": null, "rhythm": "punchy"});
        let m = merge_opt(&base, &over);
        assert_eq!(m["register"], "witty");
        assert_eq!(m["rhythm"], "punchy");
        assert_eq!(m["emoji"], "sparing");
    }

    #[test]
    fn effective_voice_account_overrides_base() {
        let base = json!({"register": "authoritative", "formality": 0.8});
        let over = json!({"register": "witty"});
        let eff = effective_voice(&base, &over);
        assert_eq!(eff["register"], "witty");
        assert_eq!(eff["formality"], 0.8);
    }

    #[test]
    fn compile_prompt_contains_rules_and_exemplars() {
        let p = json!({
            "register": "witty-authoritative",
            "avoid_words": ["leverage", "synergy"],
            "banned_moves": ["hard_sell"],
            "signature_phrases": ["ship it"],
            "exemplars": [{"text": "Short punchy hook.", "why": "punchy hook"}],
        });
        let block = compile_voice_prompt(&p, 3);
        assert!(block.contains("witty-authoritative"));
        assert!(block.contains("avoid_words"));
        assert!(block.contains("leverage"));
        assert!(block.contains("Short punchy hook."));
        assert!(compile_voice_prompt(&json!({}), 3).is_empty());
    }

    #[test]
    fn violations_catch_avoid_word_and_banned_move() {
        let p = json!({"avoid_words": ["leverage"], "banned_moves": ["hard_sell"]});
        let v = voice_violations(&p, "Leverage your synergy, 限时优惠 click now");
        let codes: Vec<&str> = v.iter().map(|x| x.code.as_str()).collect();
        assert!(codes.contains(&"voice_avoid_word"));
        assert!(codes.contains(&"voice_banned_move"));
        assert!(voice_violations(&p, "clean post").is_empty());
    }

    #[test]
    fn parse_distill_extracts_json_and_marks_provenance() {
        let raw = "Here you go:\n{\"register\":\"dry\",\"exemplars\":[{\"text\":\"a\"}]}\n";
        let v = parse_distill(raw, "test-model").unwrap();
        assert_eq!(v["register"], "dry");
        assert_eq!(v["distill_model"], "test-model");
        assert!(v["distilled_at"].is_string());
        assert!(parse_distill("no json", "m").is_err());
    }
}
