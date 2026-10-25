//! Auto-memory: small beliefs about the user, learned after a turn
//! (2026-10-25).
//!
//! Idea adapted from Mercury Agent's memory extraction (MIT),
//! reimplemented for Klyne. After a completed, non-trivial turn one small
//! model call (the configured worker model, so no extra model swap) reads
//! only the user's own words and returns a strict JSON array of at most 5
//! operations, usually `[]`. Beliefs track support/contradiction counts,
//! a confidence that decays with time, and are retired, never deleted.
//! Anything that looks like a secret or identity data is dropped. The
//! existing memory_* tools are separate and untouched.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOCK: Mutex<()> = Mutex::new(());

pub const MAX_OPS: usize = 5;
const MAX_STATEMENT: usize = 200;
const MAX_ACTIVE_IN_PROMPT: usize = 5;
/// Confidence halves every 90 days without new support.
const HALF_LIFE_DAYS: f64 = 90.0;
/// Below this (after decay) a belief is retired.
const RETIRE_BELOW: f64 = 0.2;
pub const KINDS: &[&str] = &[
    "fact",
    "preference",
    "trait",
    "situation",
    "relationship",
    "goal",
    "project",
    "episode",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Belief {
    pub id: String,
    pub kind: String,
    pub statement: String,
    pub support: u32,
    pub against: u32,
    pub confidence: f64,
    /// "active" or "retired".
    pub status: String,
    #[serde(default)]
    pub retired_reason: Option<String>,
    pub created_ms: u64,
    pub updated_ms: u64,
    #[serde(default)]
    pub source_chat: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    next: u64,
    beliefs: Vec<Belief>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn path(root: &Path) -> PathBuf {
    root.join("memory").join("beliefs.json")
}

fn load(root: &Path) -> Store {
    fs::read(path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn store(root: &Path, data: &Store) -> io::Result<()> {
    let file = path(root);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(data).map_err(io::Error::other)?,
    )?;
    fs::rename(tmp, file)
}

/// Confidence after time decay.
pub fn effective(belief: &Belief, now: u64) -> f64 {
    let days = now.saturating_sub(belief.updated_ms) as f64 / 86_400_000.0;
    belief.confidence * 0.5f64.powf(days / HALF_LIFE_DAYS)
}

fn score(support: u32, against: u32, owner_stated: bool) -> f64 {
    let base = if owner_stated { 0.8 } else { 0.55 };
    (base + 0.1 * f64::from(support.saturating_sub(1)) - 0.25 * f64::from(against)).clamp(0.0, 1.0)
}

/// All beliefs (newest first), retiring ones that decayed away.
pub fn list(root: &Path) -> io::Result<Vec<Belief>> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = load(root);
    let now = now_ms();
    let mut changed = false;
    for belief in &mut data.beliefs {
        if belief.status == "active" && effective(belief, now) < RETIRE_BELOW {
            belief.status = "retired".into();
            belief.retired_reason = Some("faded: no support for a long time".into());
            changed = true;
        }
    }
    if changed {
        store(root, &data)?;
    }
    let mut out = data.beliefs;
    out.sort_by_key(|a| std::cmp::Reverse(a.updated_ms));
    Ok(out)
}

pub fn active(root: &Path) -> Vec<Belief> {
    list(root)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.status == "active")
        .collect()
}

/// Retire (never delete) a belief, e.g. from the UI.
pub fn retire(root: &Path, id: &str, reason: &str) -> io::Result<Value> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = load(root);
    let belief = data
        .beliefs
        .iter_mut()
        .find(|b| b.id == id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Unknown belief"))?;
    belief.status = "retired".into();
    belief.retired_reason = Some(reason.chars().take(120).collect());
    belief.updated_ms = now_ms();
    let value = serde_json::to_value(&*belief).map_err(io::Error::other)?;
    store(root, &data)?;
    Ok(value)
}

/// Greetings, thanks and other short acknowledgements.
pub fn trivial(text: &str) -> bool {
    let text = text.trim().to_lowercase();
    let text = text.trim_end_matches(['!', '.', '?', ' ']);
    if text.chars().count() > 40 {
        return false;
    }
    let words = [
        "hi",
        "hello",
        "hey",
        "thanks",
        "thank you",
        "thx",
        "ok",
        "okay",
        "cool",
        "nice",
        "great",
        "yes",
        "no",
        "yep",
        "nope",
        "sure",
        "bye",
        "good",
        "perfect",
        "continue",
        "go on",
        "got it",
        "lol",
        "k",
    ];
    text.is_empty()
        || words.contains(&text)
        || text.split_whitespace().count() <= 4
            && [
                "hi",
                "hello",
                "hey",
                "thanks",
                "thank you",
                "thx",
                "bye",
                "good night",
            ]
            .iter()
            .any(|w| text.starts_with(w))
}

/// Whether the user's words could carry something worth remembering.
pub fn worth_extracting(text: &str) -> bool {
    if trivial(text) {
        return false;
    }
    let lower = format!(" {} ", text.to_lowercase());
    [
        " i ",
        " i'm ",
        " my ",
        " mine ",
        " i've ",
        " i'd ",
        " prefer",
        "remember",
        "from now on",
        "call me",
        "favorite",
        "favourite",
        " i love ",
        " i hate ",
        " i like ",
        "keep in mind",
        "note that",
    ]
    .iter()
    .any(|w| lower.contains(w))
}

/// Secrets, credentials and identity data never become beliefs.
pub fn sensitive(text: &str) -> bool {
    if crate::capabilities::sensitive_memory(text) {
        return true;
    }
    let lower = text.to_lowercase();
    let markers = [
        "token",
        "api key",
        "private key",
        "seed phrase",
        "mnemonic",
        "recovery phrase",
        "passcode",
        "pin code",
        "pin is",
        "otp",
        "2fa",
        "credential",
        "login",
        "username",
        "ssn",
        "social security",
        "passport",
        "driver's license",
        "drivers license",
        "credit card",
        "card number",
        "cvv",
        "iban",
        "bank account",
        "routing number",
        "date of birth",
        "birthday",
        "born on",
        "home address",
        "lives at",
        "live at",
        "phone number",
        "email address",
        "religion",
        "ethnicity",
        "sexual orientation",
        "diagnos",
        "medical",
    ];
    if markers.iter().any(|m| lower.contains(m)) {
        return true;
    }
    if text.contains('@') {
        return true;
    }
    // Long digit runs (phones, cards, IDs).
    let mut digits = 0;
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits += 1;
            if digits >= 7 {
                return true;
            }
        } else if !matches!(c, ' ' | '-' | '.' | '(' | ')') {
            digits = 0;
        }
    }
    // Long random-looking strings (keys, hashes, wallet addresses).
    text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';'))
        .any(|w| {
            w.len() >= 20
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '+' | '='))
                && w.chars().any(|c| c.is_ascii_digit())
                && w.chars().any(|c| c.is_ascii_alphabetic())
        })
}

pub const SYSTEM: &str = "You maintain a small memory of durable facts about the user, from the user's own words only. \
Return ONLY a JSON object {\"ops\":[...]} (no prose) whose ops array holds at most 5 operations, one object per operation; return {\"ops\":[]} for most turns. Operations: \
{\"op\":\"create\",\"kind\":K,\"statement\":S} for something new the user implied; \
{\"op\":\"owner_stated\",\"kind\":K,\"statement\":S} when the user explicitly asked you to remember it or stated it directly about themselves; \
{\"op\":\"support\",\"id\":ID} when the user's words confirm a known belief; \
{\"op\":\"contradict\",\"id\":ID} when they contradict one; \
{\"op\":\"correction\",\"id\":ID,\"statement\":S} when the user corrects one. \
K is one of fact, preference, trait, situation, relationship, goal, project, episode. \
S is one short third-person sentence starting with 'The user' (max 200 characters). \
Only durable things useful in later conversations (preferences, ongoing projects, how they like answers). \
Never store passwords, keys, tokens, codes, account names, emails, phone numbers, addresses, birth dates, health, religion or other identity data. \
Never store task details, tool output, web content or what the assistant did. If unsure, return {\"ops\":[]}. Example: {\"ops\":[{\"op\":\"owner_stated\",\"kind\":\"preference\",\"statement\":\"The user prefers metric units.\"}]}";

/// The extraction prompt: the user's words this turn plus known beliefs.
pub fn prompt(user_words: &[String], known: &[Belief]) -> String {
    let known: Vec<Value> = known
        .iter()
        .take(20)
        .map(|b| json!({"id":b.id,"kind":b.kind,"statement":b.statement}))
        .collect();
    json!({"user_words_this_turn":user_words,"known_beliefs":known}).to_string()
}

/// Parse a strict operation array (tolerating a code fence or an
/// `{"ops":[...]}` wrapper); invalid or sensitive operations are dropped.
pub fn parse_ops(text: &str) -> Vec<Value> {
    let text = text.trim();
    let value: Value = match (text.find('['), text.rfind(']')) {
        (Some(a), Some(b)) if b > a => serde_json::from_str(&text[a..=b]).unwrap_or(Value::Null),
        _ => serde_json::from_str(text).unwrap_or(Value::Null),
    };
    // JSON mode can return the wrapper, a bare array, another array key or
    // a single operation object.
    let items = value
        .as_array()
        .cloned()
        .or_else(|| value["ops"].as_array().cloned())
        .or_else(|| {
            value
                .as_object()
                .and_then(|o| o.values().find_map(|v| v.as_array().cloned()))
        })
        .or_else(|| value.get("op").map(|_| vec![value.clone()]))
        .unwrap_or_default();
    items
        .into_iter()
        .filter(|op| {
            let statement = op["statement"].as_str().unwrap_or("");
            match op["op"].as_str().unwrap_or("") {
                "create" | "owner_stated" => {
                    KINDS.contains(&op["kind"].as_str().unwrap_or(""))
                        && !statement.trim().is_empty()
                        && statement.chars().count() <= MAX_STATEMENT
                        && !sensitive(statement)
                }
                "support" | "contradict" => op["id"].is_string(),
                "correction" => {
                    op["id"].is_string()
                        && !statement.trim().is_empty()
                        && statement.chars().count() <= MAX_STATEMENT
                        && !sensitive(statement)
                }
                _ => false,
            }
        })
        .take(MAX_OPS)
        .collect()
}

/// Apply parsed operations. Returns one line per applied change.
pub fn apply(root: &Path, ops: &[Value], chat_id: &str) -> io::Result<Vec<String>> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut data = load(root);
    let now = now_ms();
    let mut applied = Vec::new();
    for op in ops.iter().take(MAX_OPS) {
        let statement = op["statement"].as_str().unwrap_or("").trim().to_owned();
        let kind = op["kind"].as_str().unwrap_or("fact").to_owned();
        let id = op["id"].as_str().unwrap_or("");
        match op["op"].as_str().unwrap_or("") {
            name @ ("create" | "owner_stated") => {
                let owner = name == "owner_stated";
                // A near-duplicate supports the existing belief instead.
                if let Some(existing) = data.beliefs.iter_mut().find(|b| {
                    b.status == "active"
                        && crate::loop_guard::similarity(&b.statement, &statement) > 0.75
                }) {
                    existing.support += 1;
                    existing.confidence =
                        score(existing.support, existing.against, owner).max(existing.confidence);
                    existing.updated_ms = now;
                    applied.push(format!("supported {}", existing.id));
                    continue;
                }
                data.next += 1;
                let belief = Belief {
                    id: format!("b{}", data.next),
                    kind,
                    statement,
                    support: 1,
                    against: 0,
                    confidence: score(1, 0, owner),
                    status: "active".into(),
                    retired_reason: None,
                    created_ms: now,
                    updated_ms: now,
                    source_chat: chat_id.into(),
                };
                applied.push(format!("created {}", belief.id));
                data.beliefs.push(belief);
            }
            "support" => {
                if let Some(b) = data
                    .beliefs
                    .iter_mut()
                    .find(|b| b.id == id && b.status == "active")
                {
                    b.support += 1;
                    b.confidence = (b.confidence + 0.1).min(1.0);
                    b.updated_ms = now;
                    applied.push(format!("supported {id}"));
                }
            }
            "contradict" => {
                if let Some(b) = data
                    .beliefs
                    .iter_mut()
                    .find(|b| b.id == id && b.status == "active")
                {
                    b.against += 1;
                    b.confidence = (b.confidence - 0.25).max(0.0);
                    b.updated_ms = now;
                    if b.against >= b.support || b.confidence < RETIRE_BELOW {
                        b.status = "retired".into();
                        b.retired_reason = Some("contradicted by the user".into());
                        applied.push(format!("retired {id}"));
                    } else {
                        applied.push(format!("contradicted {id}"));
                    }
                }
            }
            "correction" => {
                let Some(old) = data
                    .beliefs
                    .iter_mut()
                    .find(|b| b.id == id && b.status == "active")
                else {
                    continue;
                };
                old.status = "retired".into();
                old.retired_reason = Some("corrected by the user".into());
                old.updated_ms = now;
                let kind = old.kind.clone();
                data.next += 1;
                let belief = Belief {
                    id: format!("b{}", data.next),
                    kind,
                    statement,
                    support: 1,
                    against: 0,
                    confidence: score(1, 0, true),
                    status: "active".into(),
                    retired_reason: None,
                    created_ms: now,
                    updated_ms: now,
                    source_chat: chat_id.into(),
                };
                applied.push(format!("corrected {id} -> {}", belief.id));
                data.beliefs.push(belief);
            }
            _ => {}
        }
    }
    if !applied.is_empty() {
        store(root, &data)?;
    }
    Ok(applied)
}

/// Up to 5 active beliefs for the prompt's host notes: preferences first,
/// then those sharing words with the request, by confidence.
pub fn notes(root: &Path, request: &str) -> Option<String> {
    let now = now_ms();
    let request = request.to_lowercase();
    let words: Vec<&str> = request
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 3)
        .collect();
    let mut scored: Vec<(f64, Belief)> = active(root)
        .into_iter()
        .map(|b| {
            let lower = b.statement.to_lowercase();
            let overlap = words.iter().filter(|w| lower.contains(*w)).count() as f64;
            let preference = if b.kind == "preference" { 1.0 } else { 0.0 };
            (overlap + preference + effective(&b, now), b)
        })
        .filter(|(s, b)| *s >= 1.0 || b.kind == "preference")
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let lines: Vec<String> = scored
        .into_iter()
        .take(MAX_ACTIVE_IN_PROMPT)
        .map(|(_, b)| format!("{} ({})", b.statement, b.kind))
        .collect();
    (!lines.is_empty()).then(|| {
        format!(
            "Remembered about the user (may be outdated; the user's current words win): {}",
            lines.join("; ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_strict_capped_and_secret_free() {
        let text = r#"```json
[{"op":"owner_stated","kind":"preference","statement":"The user prefers short answers."},
 {"op":"create","kind":"fact","statement":"The user's password is hunter2"},
 {"op":"create","kind":"fact","statement":"The user's email is a@b.com"},
 {"op":"create","kind":"fact","statement":"The user's key is sk_live_51Habc123def456ghi789"},
 {"op":"create","kind":"bogus","statement":"x"},
 {"op":"create","kind":"project","statement":"The user is building Klyne."},
 {"op":"support","id":"b1"},{"op":"support","id":"b1"},{"op":"support","id":"b1"},
 {"op":"support","id":"b1"},{"op":"support","id":"b1"}]
```"#;
        let ops = parse_ops(text);
        assert_eq!(ops.len(), 5);
        assert_eq!(ops[0]["op"], "owner_stated");
        assert_eq!(ops[1]["kind"], "project");
        assert!(parse_ops("[]").is_empty());
        assert!(parse_ops("I think nothing").is_empty());
        assert_eq!(
            parse_ops(r#"{"ops":[{"op":"support","id":"b2"}]}"#).len(),
            1
        );
        for secret in [
            "my seed phrase is apple banana",
            "call 0901234567",
            "token abc",
            "I was born on May 1",
        ] {
            assert!(sensitive(secret), "{secret}");
        }
        assert!(!sensitive("The user prefers metric units."));
    }

    #[test]
    fn trivial_turns_are_skipped() {
        for t in ["hi", "Thanks!", "ok", "thank you so much", "continue"] {
            assert!(trivial(t) && !worth_extracting(t), "{t}");
        }
        assert!(worth_extracting("Remember that I prefer metric units"));
        assert!(!worth_extracting(
            "Summarize this GitHub issue in two sentences"
        ));
    }

    #[test]
    fn beliefs_gain_support_get_corrected_and_retire_without_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let created = apply(
            root,
            &[json!({"op":"owner_stated","kind":"preference","statement":"The user prefers short answers."})],
            "c1",
        )
        .unwrap();
        assert_eq!(created, vec!["created b1"]);
        // A near-duplicate supports instead of duplicating.
        apply(root, &[json!({"op":"create","kind":"preference","statement":"The user prefers short answers"})], "c2").unwrap();
        let all = list(root).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].support, 2);
        assert!(notes(root, "anything").unwrap().contains("short answers"));
        let out = apply(root, &[json!({"op":"correction","id":"b1","statement":"The user prefers detailed answers."})], "c3").unwrap();
        assert_eq!(out, vec!["corrected b1 -> b2"]);
        let all = list(root).unwrap();
        assert_eq!(all.len(), 2, "retired, not deleted");
        assert!(all.iter().any(|b| b.id == "b1" && b.status == "retired"));
        retire(root, "b2", "retired by the user").unwrap();
        assert!(active(root).is_empty());
        assert_eq!(list(root).unwrap().len(), 2);
        // Decay: an old belief fades and retires on read.
        let mut b = list(root).unwrap()[0].clone();
        b.confidence = 0.8;
        assert!(effective(&b, b.updated_ms + 400 * 86_400_000) < RETIRE_BELOW);
    }
}
