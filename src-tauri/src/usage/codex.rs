//! Exact token usage from Codex JSONL session logs.
//!
//! Ported from cc-switch (`services/session_usage_codex.rs`). cc-switch imports
//! incrementally into SQLite; dbox has no database, so every scan re-reads the
//! rollout files (with a per-file parse cache keyed by size + mtime) and
//! deduplicates requests in memory by the same per-thread request keys.
//!
//! ## Parsed records
//! - `session_meta` → the thread ID and an optional parent (sub-agent / fork)
//! - `turn_context` / `event_msg` (type=thread_settings_applied) → current model
//! - `event_msg` (type=token_count) → per-request usage (`last_token_usage`),
//!   falling back to the delta of the cumulative `total_token_usage`
//! - `token_usage_record` → billed when the following `token_count` has an
//!   all-zero `last_token_usage` (remote compaction)
//!
//! Sub-agent and forked rollouts start by replaying the parent's token events.
//! That replayed prefix is matched against the parent timeline and skipped, so
//! parent usage is never counted twice. When the parent cannot be resolved
//! reliably the child is reported as deferred instead of guessed.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::rollout_file;

/// Cumulative token usage (`total_token_usage`) or one request's usage.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CumulativeTokens {
    input: u64,
    cached_input: u64,
    cache_write_input: u64,
    output: u64,
}

/// Token usage of one API request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeltaTokens {
    pub input: u64,
    pub cached_input: u64,
    pub cache_write_input: u64,
    pub output: u64,
}

impl DeltaTokens {
    fn is_zero(&self) -> bool {
        self.input == 0 && self.cached_input == 0 && self.output == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenCountersSignature {
    input: Option<u64>,
    cached_input: Option<u64>,
    output: Option<u64>,
    reasoning_output: Option<u64>,
    total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenUsageSignature {
    total: Option<TokenCountersSignature>,
    last: Option<TokenCountersSignature>,
}

#[derive(Debug, Clone)]
struct TimestampedTokenSignature {
    timestamp: DateTime<Utc>,
    signature: TokenUsageSignature,
}

/// Size + mtime identity used to detect changed rollouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    /// Modification time in nanoseconds since the Unix epoch.
    pub modified_ns: Option<u64>,
    pub size: u64,
}

impl FileStamp {
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            modified_ns: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                .and_then(|duration| u64::try_from(duration.as_nanos()).ok()),
            size: metadata.len(),
        })
    }
}

#[derive(Debug)]
struct ParentTokenTimeline {
    events: Vec<TimestampedTokenSignature>,
    max_timestamp: Option<DateTime<Utc>>,
    has_token_without_timestamp: bool,
    high_water: Option<CumulativeTokens>,
}

impl ParentTokenTimeline {
    fn signatures_before(
        &self,
        parent_path: &Path,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<TokenUsageSignature>, String> {
        if self.has_token_without_timestamp {
            return Err(format!(
                "父会话 {} 的 token_count 缺少有效 timestamp",
                parent_path.display()
            ));
        }
        if self
            .max_timestamp
            .is_none_or(|timestamp| timestamp < cutoff)
        {
            return Err(format!(
                "父会话 {} 尚未写到子会话 fork 时刻",
                parent_path.display()
            ));
        }
        Ok(self.signatures_at(cutoff))
    }

    fn signatures_at(&self, cutoff: DateTime<Utc>) -> Vec<TokenUsageSignature> {
        self.events
            .iter()
            .filter(|event| event.timestamp <= cutoff)
            .map(|event| event.signature.clone())
            .collect()
    }
}

#[derive(Debug)]
struct ParsedTokenEvent {
    signature: TokenUsageSignature,
    delta: DeltaTokens,
    /// 1-based index among non-zero `token_count` events of this file.
    event_index: Option<u32>,
    /// Usage that does not take a `token_count` index (remote compaction).
    request_suffix: Option<String>,
    model: String,
    timestamp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParentResolution {
    None,
    Parent(String),
    Continuation,
    Deferred(String),
}

#[derive(Debug)]
struct ParsedCodexFile {
    root_thread_id: Option<String>,
    /// Thread ID from the root `session_meta` (stable logical thread).
    meta_thread_id: Option<String>,
    root_meta_seen: bool,
    bounded_history: Option<serde_json::Value>,
    root_timestamp: Option<DateTime<Utc>>,
    parent: ParentResolution,
    token_events: Vec<ParsedTokenEvent>,
    usage_record_events: Vec<ParsedTokenEvent>,
    has_billable_tokens: bool,
    /// The result depends on another rollout (continuation baseline), so it
    /// must not be cached by this file's stamp alone.
    depends_on_other_files: bool,
}

type RolloutIndex = HashMap<String, Vec<PathBuf>>;

/// Parse cache kept across scans by the usage service.
#[derive(Debug, Default)]
pub struct CodexParseCache {
    parsed: HashMap<PathBuf, (FileStamp, Arc<ParsedCodexFile>)>,
    timelines: HashMap<PathBuf, (FileStamp, Arc<ParentTokenTimeline>)>,
}

impl CodexParseCache {
    fn retain_paths(&mut self, files: &HashSet<&PathBuf>) {
        self.parsed.retain(|path, _| files.contains(path));
        self.timelines.retain(|path, _| files.contains(path));
    }
}

/// One counted request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexUsageEvent {
    /// Stable request identity: `<rollout thread ID>:<event index | usage:id>`.
    pub request_key: String,
    pub thread_id: String,
    pub model: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub tokens: DeltaTokens,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredFile {
    pub path: PathBuf,
    pub reason: String,
}

/// A rollout that was parsed in this scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub stamp: Option<FileStamp>,
    /// Deferred files must be parsed again on the next scan.
    pub deferred: bool,
    pub replayed_events: usize,
    pub duplicate_events: usize,
}

#[derive(Debug, Default)]
pub struct CodexScan {
    pub files_scanned: usize,
    /// Rollouts parsed in this scan; unchanged rollouts the caller skipped are absent.
    pub files: Vec<ScannedFile>,
    pub events: Vec<CodexUsageEvent>,
    pub deferred: Vec<DeferredFile>,
    /// Token events skipped because they replay parent history.
    pub replayed_events: usize,
    /// Requests seen again in another copy of the same rollout.
    pub duplicate_events: usize,
}

fn non_empty_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn canonical_uuid(id: String) -> String {
    uuid::Uuid::parse_str(&id)
        .map(|value| value.hyphenated().to_string())
        .unwrap_or(id)
}

fn thread_id_from_filename(path: &Path) -> Option<String> {
    let logical = rollout_file::logical_path(path);
    let stem = logical.file_stem()?.to_str()?;
    let candidate = stem.get(stem.len().checked_sub(36)?..)?;
    uuid::Uuid::parse_str(candidate)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

/// Leading thread UUID of a two-part file name
/// (`rollout-…-<threadId>_<rolloutId>.jsonl`) written by `thread/revert` and
/// paginated history; `None` for single-UUID names.
fn leading_thread_id_from_filename(path: &Path) -> Option<String> {
    let logical = rollout_file::logical_path(path);
    let stem = logical.file_stem()?.to_str()?;
    let len = stem.len();
    if !stem.get(len.checked_sub(37)?..)?.starts_with('_') {
        return None;
    }
    let candidate = stem.get(len.checked_sub(73)?..len.checked_sub(37)?)?;
    uuid::Uuid::parse_str(candidate)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

fn explicit_parent_from_meta(payload: &serde_json::Value) -> ParentResolution {
    let forked_from = non_empty_string(payload.get("forked_from_id"));
    let spawned_from = payload
        .get("source")
        .and_then(|source| source.get("subagent"))
        .and_then(|subagent| subagent.get("thread_spawn"))
        .and_then(|spawn| non_empty_string(spawn.get("parent_thread_id")));

    match (forked_from, spawned_from) {
        (None, None) => ParentResolution::None,
        (Some(parent), None) | (None, Some(parent)) => ParentResolution::Parent(parent),
        (Some(forked), Some(spawned)) if forked == spawned => ParentResolution::Parent(forked),
        (Some(forked), Some(spawned)) => ParentResolution::Deferred(format!(
            "forked_from_id ({forked}) 与 thread_spawn.parent_thread_id ({spawned}) 不一致"
        )),
    }
}

fn parse_timestamp(value: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(serde_json::Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn parse_signature_counters(value: Option<&serde_json::Value>) -> Option<TokenCountersSignature> {
    let value = value?.as_object()?;
    let field = |name: &str| value.get(name).and_then(serde_json::Value::as_u64);
    Some(TokenCountersSignature {
        input: field("input_tokens"),
        cached_input: field("cached_input_tokens").or_else(|| field("cache_read_input_tokens")),
        output: field("output_tokens"),
        reasoning_output: field("reasoning_output_tokens"),
        total: field("total_tokens"),
    })
}

fn parse_token_signature(info: &serde_json::Value) -> Option<TokenUsageSignature> {
    let total = parse_signature_counters(info.get("total_token_usage"));
    let last = parse_signature_counters(info.get("last_token_usage"));
    (total.is_some() || last.is_some()).then_some(TokenUsageSignature { total, last })
}

fn token_snapshot_source(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("rate_limits")
        .and_then(|rate_limits| rate_limits.get("limit_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Normalize a Codex model name so equivalent names aggregate together.
///
/// Rules, in order:
/// 1. lowercase: `GLM-4.6` → `glm-4.6`
/// 2. strip a provider prefix: `openai/gpt-5.4` → `gpt-5.4`
/// 3. strip an ISO date suffix: `gpt-5.4-2026-03-05` → `gpt-5.4`
/// 4. strip a compact date suffix: `gpt-5.4-20260305` → `gpt-5.4`
pub fn normalize_codex_model(raw: &str) -> String {
    let mut name = raw.to_lowercase();

    if let Some(pos) = name.rfind('/') {
        name = name[pos + 1..].to_string();
    }

    // -YYYY-MM-DD (exactly 11 characters)
    if name.len() > 11 && name.is_char_boundary(name.len() - 11) {
        let suffix = &name[name.len() - 11..];
        let bytes = suffix.as_bytes();
        if suffix.is_ascii()
            && bytes[0] == b'-'
            && suffix[1..5].chars().all(|c| c.is_ascii_digit())
            && bytes[5] == b'-'
            && suffix[6..8].chars().all(|c| c.is_ascii_digit())
            && bytes[8] == b'-'
            && suffix[9..11].chars().all(|c| c.is_ascii_digit())
        {
            name.truncate(name.len() - 11);
        }
    }

    // -YYYYMMDD (exactly 9 characters)
    if name.len() > 9 {
        if let Some((head, suffix)) = name.rsplit_once('-') {
            if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
                name = head.to_string();
            }
        }
    }

    name
}

fn compute_delta(prev: &Option<CumulativeTokens>, current: &CumulativeTokens) -> DeltaTokens {
    match prev {
        None => DeltaTokens {
            input: current.input,
            cached_input: current.cached_input,
            cache_write_input: current.cache_write_input,
            output: current.output,
        },
        Some(p) => DeltaTokens {
            input: current.input.saturating_sub(p.input),
            cached_input: current.cached_input.saturating_sub(p.cached_input),
            cache_write_input: current
                .cache_write_input
                .saturating_sub(p.cache_write_input),
            output: current.output.saturating_sub(p.output),
        },
    }
}

fn usage_is_zero(usage: &CumulativeTokens) -> bool {
    usage == &CumulativeTokens::default()
}

fn delta_from_usage(usage: &CumulativeTokens) -> DeltaTokens {
    let cached_input = usage.cached_input.min(usage.input);
    let fresh_input = usage.input - cached_input;
    DeltaTokens {
        input: usage.input,
        cached_input,
        cache_write_input: usage.cache_write_input.min(fresh_input),
        output: usage.output,
    }
}

fn signature_for_last_usage(usage: &CumulativeTokens) -> TokenUsageSignature {
    TokenUsageSignature {
        total: None,
        last: Some(TokenCountersSignature {
            input: Some(usage.input),
            cached_input: Some(usage.cached_input),
            output: Some(usage.output),
            reasoning_output: None,
            total: Some(usage.input.saturating_add(usage.output)),
        }),
    }
}

struct PendingUsageRecord {
    key: String,
    usage: CumulativeTokens,
    timestamp: Option<String>,
    model: String,
}

fn push_unmatched_usage(
    events: &mut Vec<ParsedTokenEvent>,
    pending: PendingUsageRecord,
    has_billable_tokens: &mut bool,
) {
    let delta = delta_from_usage(&pending.usage);
    if delta.is_zero() {
        return;
    }
    *has_billable_tokens = true;
    events.push(ParsedTokenEvent {
        signature: signature_for_last_usage(&pending.usage),
        delta,
        event_index: None,
        request_suffix: Some(format!("usage:{}", pending.key)),
        model: pending.model,
        timestamp: pending.timestamp,
    });
}

fn update_high_water(high_water: &mut CumulativeTokens, current: &CumulativeTokens) {
    high_water.input = high_water.input.max(current.input);
    high_water.cached_input = high_water.cached_input.max(current.cached_input);
    high_water.cache_write_input = high_water.cache_write_input.max(current.cache_write_input);
    high_water.output = high_water.output.max(current.output);
}

fn parse_cumulative_tokens(usage: &serde_json::Value) -> Option<CumulativeTokens> {
    let fields = usage.as_object()?;
    if ![
        "input_tokens",
        "cached_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
        "total_tokens",
    ]
    .iter()
    .any(|field| fields.contains_key(*field))
    {
        return None;
    }
    let field = |name: &str| usage.get(name).and_then(serde_json::Value::as_u64);
    Some(CumulativeTokens {
        input: field("input_tokens").unwrap_or(0),
        cached_input: field("cached_input_tokens")
            .or_else(|| field("cache_read_input_tokens"))
            .unwrap_or(0),
        cache_write_input: field("cache_write_input_tokens").unwrap_or(0),
        output: field("output_tokens").unwrap_or(0),
    })
}

/// Scan `sessions/YYYY/MM/DD/*.jsonl[.zst]` and `archived_sessions/*`.
fn collect_codex_session_files(codex_home: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let sessions_dir = codex_home.join("sessions");
    if sessions_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&sessions_dir)
            .max_depth(4)
            .into_iter()
            .flatten()
        {
            if entry.file_type().is_file() && rollout_file::is_rollout_file(entry.path()) {
                files.push(entry.into_path());
            }
        }
    }
    let archived_dir = codex_home.join("archived_sessions");
    if let Ok(entries) = fs::read_dir(&archived_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && rollout_file::is_rollout_file(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    rollout_file::dedupe_siblings(files)
}

fn build_rollout_index(files: &[PathBuf]) -> RolloutIndex {
    let mut index = RolloutIndex::new();
    for path in files {
        if let Some(thread_id) = thread_id_from_filename(path) {
            index.entry(thread_id).or_default().push(path.clone());
        }
        // Continuation pages (`<threadId>_<rolloutId>`) are also indexed under
        // the thread UUID so a parent's later pages are part of its timeline.
        if let Some(thread_id) = leading_thread_id_from_filename(path) {
            index.entry(thread_id).or_default().push(path.clone());
        }
    }
    for paths in index.values_mut() {
        paths.sort();
        paths.dedup();
    }
    index
}

/// Read (or reuse) the full token timeline of one rollout file.
fn load_parent_timeline(
    parent_path: &Path,
    cache: &mut HashMap<PathBuf, (FileStamp, Arc<ParentTokenTimeline>)>,
) -> Result<Arc<ParentTokenTimeline>, String> {
    let stamp = FileStamp::of(parent_path);
    if let Some((cached_stamp, timeline)) = cache.get(parent_path) {
        if Some(*cached_stamp) == stamp {
            return Ok(Arc::clone(timeline));
        }
    }
    let file = fs::File::open(parent_path)
        .map_err(|error| format!("无法打开父会话 {}: {error}", parent_path.display()))?;
    let mut reader = rollout_file::reader_from_file(parent_path, file)
        .map_err(|error| format!("无法打开父会话 {}: {error}", parent_path.display()))?;

    let mut events = Vec::new();
    let mut max_timestamp: Option<DateTime<Utc>> = None;
    let mut has_token_without_timestamp = false;
    let mut high_water: Option<CumulativeTokens> = None;
    // Scan the whole file: rollout timestamps are not strictly monotonic.
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let read = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|error| format!("无法读取父会话 {}: {error}", parent_path.display()))?;
        if read == 0 {
            break;
        }
        let Ok(line) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let timestamp = parse_timestamp(value.get("timestamp"));
        if let Some(timestamp) = timestamp {
            max_timestamp = Some(max_timestamp.map_or(timestamp, |current| current.max(timestamp)));
        }
        if value.get("type").and_then(serde_json::Value::as_str) != Some("event_msg")
            || value
                .pointer("/payload/type")
                .and_then(serde_json::Value::as_str)
                != Some("token_count")
        {
            continue;
        }
        let Some(info) = value
            .pointer("/payload/info")
            .filter(|info| !info.is_null())
        else {
            continue;
        };
        let Some(signature) = parse_token_signature(info) else {
            continue;
        };
        let Some(timestamp) = timestamp else {
            has_token_without_timestamp = true;
            continue;
        };
        events.push(TimestampedTokenSignature {
            timestamp,
            signature,
        });
        if let Some(total) = info
            .get("total_token_usage")
            .and_then(parse_cumulative_tokens)
        {
            match high_water.as_mut() {
                Some(hw) => update_high_water(hw, &total),
                None => high_water = Some(total),
            }
        }
    }

    let timeline = Arc::new(ParentTokenTimeline {
        events,
        max_timestamp,
        has_token_without_timestamp,
        high_water,
    });
    if let Some(stamp) = stamp {
        cache.insert(parent_path.to_path_buf(), (stamp, Arc::clone(&timeline)));
    }
    Ok(timeline)
}

/// Cumulative high-water mark of the segment preceding a continuation page.
///
/// Without it, a continuation page whose `token_count` lacks
/// `last_token_usage` would bill the whole history as its first delta.
fn resolve_continuation_baseline(
    file_path: &Path,
    meta_thread_id: &str,
    history_base_id: Option<&str>,
    rollout_index: &RolloutIndex,
    timelines: &mut HashMap<PathBuf, (FileStamp, Arc<ParentTokenTimeline>)>,
) -> Result<Option<CumulativeTokens>, String> {
    let current_file_name = file_path.file_name();
    let mut thread_ids = vec![meta_thread_id];
    if let Some(hb_id) = history_base_id.filter(|id| *id != meta_thread_id) {
        thread_ids.push(hb_id);
    }
    for thread_id in thread_ids {
        let Some(candidates) = rollout_index.get(thread_id) else {
            continue;
        };
        for prev_path in candidates.iter().rev() {
            if prev_path.file_name() < current_file_name {
                let timeline = load_parent_timeline(prev_path, timelines)
                    .map_err(|e| format!("无法读取续写页前序分段: {e}"))?;
                if let Some(hw) = timeline.high_water.clone() {
                    return Ok(Some(hw));
                }
            }
        }
    }
    if history_base_id.is_some() {
        return Err(format!(
            "找不到续写页 history_base 对应的前序分段 (meta: {meta_thread_id})"
        ));
    }
    Ok(None)
}

fn parse_codex_file(
    file_path: &Path,
    root_thread_id: Option<String>,
    rollout_index: &RolloutIndex,
    timelines: &mut HashMap<PathBuf, (FileStamp, Arc<ParentTokenTimeline>)>,
) -> Result<ParsedCodexFile, String> {
    let file = fs::File::open(file_path).map_err(|e| format!("无法打开文件: {e}"))?;
    let mut reader = rollout_file::reader_from_file(file_path, file)
        .map_err(|e| format!("无法打开文件: {e}"))?;
    let mut root_meta_seen = false;
    let mut bounded_history = None;
    let mut root_timestamp = None;
    let mut meta_thread_id: Option<String> = None;
    let mut parent = ParentResolution::None;
    let mut current_model = "unknown".to_string();
    // `total_token_usage` is session-cumulative across model and rate-limit
    // bucket changes; exact `last_token_usage` is preferred when present.
    let mut total_high_water = None;
    // Rate-limit refreshes can re-emit unchanged token info under another
    // `limit_id`. Same-source repeats match that source's latest snapshot;
    // cross-source repeats must match the immediately preceding token event.
    let mut last_signature_by_source: HashMap<Option<String>, TokenUsageSignature> = HashMap::new();
    let mut previous_token_signature = None;
    let mut is_continuation = false;
    let mut history_base_thread_id: Option<String> = None;
    let mut depends_on_other_files = false;
    let mut event_index = 0u32;
    let mut token_events = Vec::new();
    let mut usage_record_events = Vec::new();
    let mut pending_usage: Option<PendingUsageRecord> = None;
    let mut line_number = 0u64;
    let mut has_billable_tokens = false;

    loop {
        let mut bytes = Vec::new();
        let read = reader
            .read_until(b'\n', &mut bytes)
            .map_err(|e| format!("无法读取 Codex 日志: {e}"))?;
        // A live writer may have written only part of the final record.
        if read == 0
            || (bytes.last() != Some(&b'\n')
                && serde_json::from_slice::<serde_json::Value>(&bytes).is_err())
        {
            break;
        }
        line_number += 1;
        let Ok(line) = String::from_utf8(bytes) else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }

        // Cheap pre-filter: most lines (tool output, messages) are irrelevant.
        let is_event_msg = line.contains("\"event_msg\"");
        let is_turn_context = line.contains("\"turn_context\"");
        let is_session_meta = line.contains("\"session_meta\"");
        let is_usage_record = line.contains("\"token_usage_record\"");
        if !is_event_msg && !is_turn_context && !is_session_meta && !is_usage_record {
            continue;
        }
        if is_event_msg
            && !line.contains("\"token_count\"")
            && !line.contains("\"thread_settings_applied\"")
        {
            continue;
        }

        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(event_type) = value.get("type").and_then(serde_json::Value::as_str) else {
            continue;
        };

        match event_type {
            "session_meta" if !root_meta_seen => {
                root_meta_seen = true;
                root_timestamp = parse_timestamp(value.get("timestamp"));
                let payload = value.get("payload").unwrap_or(&serde_json::Value::Null);
                parent = explicit_parent_from_meta(payload);
                bounded_history = payload.get("history_base").cloned();
                meta_thread_id = non_empty_string(
                    payload
                        .get("id")
                        .or_else(|| payload.get("thread_id"))
                        .or_else(|| payload.get("threadId")),
                )
                .map(canonical_uuid);
                let leading_id = leading_thread_id_from_filename(file_path);
                if let (Some(filename_id), Some(meta_id)) =
                    (&root_thread_id, meta_thread_id.as_ref())
                {
                    let matches =
                        filename_id == meta_id || leading_id.as_deref() == Some(meta_id.as_str());
                    if !matches {
                        parent = ParentResolution::Deferred(format!(
                            "文件名线程 ID ({filename_id}) 与 session_meta ID ({meta_id}) 不一致"
                        ));
                    }
                }

                // Continuation pages keep the branch's forked_from_id, but they
                // hold events appended after pagination and do not replay
                // parent usage. Treat them as the same thread, not as a fork.
                history_base_thread_id =
                    non_empty_string(value.pointer("/payload/history_base/thread_id"))
                        .map(canonical_uuid);
                let is_paginated = payload
                    .get("history_mode")
                    .and_then(serde_json::Value::as_str)
                    == Some("paginated");
                let has_history_base = history_base_thread_id.is_some();
                let has_parent = matches!(parent, ParentResolution::Parent(_));
                is_continuation = match (leading_id.as_deref(), meta_thread_id.as_deref()) {
                    (Some(leading), Some(meta)) if leading == meta => {
                        is_paginated || has_history_base || has_parent
                    }
                    _ => {
                        has_history_base
                            && history_base_thread_id.as_deref() == meta_thread_id.as_deref()
                    }
                };
                if is_continuation && !matches!(parent, ParentResolution::Deferred(_)) {
                    parent = ParentResolution::Continuation;
                }

                if let ParentResolution::Parent(parent_id) = &mut parent {
                    match uuid::Uuid::parse_str(parent_id) {
                        Ok(value) => *parent_id = value.hyphenated().to_string(),
                        Err(_) => {
                            parent = ParentResolution::Deferred(format!(
                                "parent_thread_id 不是有效 UUID: {parent_id}"
                            ));
                        }
                    }
                }
                if matches!((&root_thread_id, &parent), (Some(root), ParentResolution::Parent(parent_id)) if root == parent_id)
                {
                    parent =
                        ParentResolution::Deferred("parent_thread_id 与自身线程 ID 相同".into());
                }
            }
            "turn_context" => {
                if let Some(model) = value
                    .pointer("/payload/model")
                    .or_else(|| value.pointer("/payload/info/model"))
                    .and_then(serde_json::Value::as_str)
                {
                    current_model = normalize_codex_model(model);
                }
            }
            "event_msg" => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                let payload_type = payload.get("type").and_then(serde_json::Value::as_str);
                if payload_type == Some("thread_settings_applied") {
                    // Resumed/Desktop threads may switch models without a new
                    // turn_context. Only the attribution changes.
                    if let Some(model) = payload
                        .pointer("/thread_settings/model")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|model| !model.is_empty())
                    {
                        current_model = normalize_codex_model(model);
                    }
                    continue;
                }
                if payload_type != Some("token_count") {
                    continue;
                }
                let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                    continue;
                };
                let Some(signature) = parse_token_signature(info) else {
                    continue;
                };
                if let Some(model) = info
                    .get("model")
                    .or_else(|| info.get("model_name"))
                    .or_else(|| payload.get("model"))
                    .and_then(serde_json::Value::as_str)
                {
                    current_model = normalize_codex_model(model);
                }

                let snapshot_source = token_snapshot_source(payload);
                let total = info
                    .get("total_token_usage")
                    .and_then(parse_cumulative_tokens);
                let last = info
                    .get("last_token_usage")
                    .and_then(parse_cumulative_tokens);
                if let Some(pending) = pending_usage.take() {
                    // A matching `last` already bills this usage. An all-zero
                    // `last` follows a remote compaction whose usage was only
                    // written to token_usage_record.
                    let covered = last.as_ref() == Some(&pending.usage);
                    if !covered && last.as_ref().is_some_and(usage_is_zero) {
                        push_unmatched_usage(
                            &mut usage_record_events,
                            pending,
                            &mut has_billable_tokens,
                        );
                    }
                }
                if total.is_none() && last.is_none() {
                    continue;
                }
                let has_total_snapshot = total.is_some();
                let duplicate_snapshot = has_total_snapshot
                    && (last_signature_by_source.get(&snapshot_source) == Some(&signature)
                        || previous_token_signature.as_ref() == Some(&signature));
                if has_total_snapshot {
                    last_signature_by_source.insert(snapshot_source, signature.clone());
                }
                previous_token_signature = Some(signature.clone());

                let delta = if duplicate_snapshot {
                    DeltaTokens::default()
                } else if let Some(last) = last {
                    // Exact per-request usage; cumulative snapshots may come
                    // from independently advancing rate-limit lanes.
                    DeltaTokens {
                        input: last.input,
                        cached_input: last.cached_input,
                        cache_write_input: last.cache_write_input,
                        output: last.output,
                    }
                } else if let Some(total) = total.as_ref() {
                    if is_continuation && total_high_water.is_none() {
                        depends_on_other_files = true;
                        let meta_id = meta_thread_id.as_deref().unwrap_or_default();
                        match resolve_continuation_baseline(
                            file_path,
                            meta_id,
                            history_base_thread_id.as_deref(),
                            rollout_index,
                            timelines,
                        ) {
                            // A smaller counter means the page restarted counting.
                            Ok(Some(baseline)) if total.input >= baseline.input => {
                                total_high_water = Some(baseline);
                            }
                            Ok(_) => total_high_water = None,
                            Err(err) => {
                                parent = ParentResolution::Deferred(format!(
                                    "续写页缺少可靠的前序累计用量基线: {err}"
                                ));
                                has_billable_tokens = true;
                                break;
                            }
                        }
                    }
                    compute_delta(&total_high_water, total)
                } else {
                    continue;
                };
                if let Some(total) = total {
                    match total_high_water.as_mut() {
                        Some(high_water) => update_high_water(high_water, &total),
                        None => total_high_water = Some(total),
                    }
                }
                // Cache reads and writes are disjoint parts of input_tokens.
                let cached_input = delta.cached_input.min(delta.input);
                let delta = DeltaTokens {
                    cached_input,
                    cache_write_input: delta.cache_write_input.min(delta.input - cached_input),
                    ..delta
                };
                let nonzero_index = if delta.is_zero() {
                    None
                } else {
                    has_billable_tokens = true;
                    event_index = event_index.saturating_add(1);
                    Some(event_index)
                };
                token_events.push(ParsedTokenEvent {
                    signature,
                    delta,
                    event_index: nonzero_index,
                    request_suffix: None,
                    model: current_model.clone(),
                    timestamp: value
                        .get("timestamp")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                });
            }
            "token_usage_record" => {
                pending_usage = None;
                let Some(usage) = value
                    .pointer("/payload/usage")
                    .and_then(parse_cumulative_tokens)
                else {
                    continue;
                };
                if usage_is_zero(&usage) {
                    continue;
                }
                let key = value
                    .pointer("/payload/response_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map_or_else(|| format!("line{line_number}"), str::to_owned);
                pending_usage = Some(PendingUsageRecord {
                    key,
                    usage,
                    timestamp: value
                        .get("timestamp")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    model: current_model.clone(),
                });
            }
            _ => {}
        }
    }

    Ok(ParsedCodexFile {
        root_thread_id,
        meta_thread_id,
        root_meta_seen,
        bounded_history,
        root_timestamp,
        parent,
        token_events,
        usage_record_events,
        has_billable_tokens,
        depends_on_other_files,
    })
}

/// One parent segment (first page or a continuation page), possibly copied.
struct ParentSegment {
    rollout_id: Option<String>,
    timeline: Arc<ParentTokenTimeline>,
    max_timestamp: Option<DateTime<Utc>>,
}

fn resolve_parent_signatures(
    parent_id: &str,
    cutoff: DateTime<Utc>,
    rollout_index: &RolloutIndex,
    timelines: &mut HashMap<PathBuf, (FileStamp, Arc<ParentTokenTimeline>)>,
) -> Result<Vec<TokenUsageSignature>, String> {
    let Some(candidates) = rollout_index.get(parent_id) else {
        return Err(format!("找不到父会话: {parent_id}"));
    };
    // Copies of one rollout (sessions/ and archived_sessions/) must agree;
    // continuation pages are later segments merged in file-name order.
    let mut ordered = candidates.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    let mut merged = ParentTokenTimeline {
        events: Vec::new(),
        max_timestamp: None,
        has_token_without_timestamp: false,
        high_water: None,
    };
    let mut segments: Vec<ParentSegment> = Vec::new();
    let mut latest_path: Option<&Path> = None;
    for candidate in ordered {
        let timeline = load_parent_timeline(candidate, timelines)?;
        merged.has_token_without_timestamp |= timeline.has_token_without_timestamp;
        let rollout_id = thread_id_from_filename(candidate);
        if let Some(segment) = segments
            .iter_mut()
            .find(|segment| segment.rollout_id == rollout_id)
        {
            if segment.timeline.signatures_at(cutoff) != timeline.signatures_at(cutoff) {
                return Err(format!("父会话 {parent_id} 对应多个内容不一致的文件"));
            }
            // Every copy must have reached the fork time.
            segment.max_timestamp = match (segment.max_timestamp, timeline.max_timestamp) {
                (Some(current), Some(other)) => Some(current.min(other)),
                _ => None,
            };
            continue;
        }
        merged.events.extend(timeline.events.iter().cloned());
        segments.push(ParentSegment {
            rollout_id,
            max_timestamp: timeline.max_timestamp,
            timeline,
        });
        latest_path = Some(candidate.as_path());
    }
    merged.max_timestamp = segments
        .iter()
        .filter_map(|segment| segment.max_timestamp)
        .max();
    let Some(latest_path) = latest_path else {
        return Err(format!("找不到父会话: {parent_id}"));
    };
    merged.signatures_before(latest_path, cutoff)
}

/// Number of leading child token events that replay the parent's history,
/// matched as a subsequence of the parent signatures.
fn matching_replay_prefix(child: &[ParsedTokenEvent], parent: &[TokenUsageSignature]) -> usize {
    let mut parent_offset = 0usize;
    let mut matched = 0usize;
    for event in child {
        let Some(relative_match) = parent[parent_offset..]
            .iter()
            .position(|signature| signature == &event.signature)
        else {
            break;
        };
        parent_offset += relative_match + 1;
        matched += 1;
    }
    matched
}

/// A fork with a verifiable `history_base` that references the parent by byte
/// range instead of copying its events: all child usage is new.
fn verified_bounded_fork(parsed: &ParsedCodexFile, index: &RolloutIndex) -> bool {
    (|| {
        let ParentResolution::Parent(parent_id) = &parsed.parent else {
            return None;
        };
        let hb = parsed.bounded_history.as_ref()?;
        if hb.get("thread_id")?.as_str()? != parent_id {
            return None;
        }
        let end = hb.get("end_byte_offset")?.as_u64()?;
        let count = hb.get("end_ordinal_exclusive")?.as_u64()?;
        if end == 0 || end > 64 * 1024 * 1024 || count == 0 {
            return None;
        }
        let cutoff = parsed.root_timestamp?;
        let first = parsed.token_events.first()?;
        // Reject copied history, cumulative-only accounting and events that
        // are not after the fork.
        for event in parsed
            .token_events
            .iter()
            .chain(&parsed.usage_record_events)
        {
            if event.signature.last.as_ref()?.input.is_none()
                || event.signature.last.as_ref()?.output.is_none()
                || parse_timestamp(Some(&serde_json::Value::String(event.timestamp.clone()?)))?
                    <= cutoff
            {
                return None;
            }
        }
        let total = first.signature.total.as_ref()?;
        let last = first.signature.last.as_ref()?;
        let baseline = TokenCountersSignature {
            input: Some(total.input?.checked_sub(last.input?)?),
            cached_input: Some(total.cached_input?.checked_sub(last.cached_input?)?),
            output: Some(total.output?.checked_sub(last.output?)?),
            reasoning_output: Some(
                total
                    .reasoning_output?
                    .checked_sub(last.reasoning_output?)?,
            ),
            total: Some(total.total?.checked_sub(last.total?)?),
        };
        let mut found = false;
        for path in index.get(parent_id)? {
            if thread_id_from_filename(path).as_deref() != Some(parent_id) {
                continue;
            }
            let stamp = FileStamp::of(path)?;
            let file = fs::File::open(path).ok()?;
            let mut reader = rollout_file::reader_from_file(path, file).ok()?;
            let mut used = 0u64;
            let mut ordinal = 0u64;
            let mut first_record = true;
            let mut parent_total = None;
            while used < end {
                let mut bytes = Vec::new();
                let n = reader
                    .by_ref()
                    .take(end - used)
                    .read_until(b'\n', &mut bytes)
                    .ok()?;
                if n == 0 || bytes.last() != Some(&b'\n') {
                    return None;
                }
                used += n as u64;
                let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
                if first_record {
                    ordinal = value.get("ordinal")?.as_u64()?;
                    let inherited = value
                        .pointer("/payload/history_base/end_ordinal_exclusive")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if ordinal != inherited
                        || value.get("type")?.as_str()? != "session_meta"
                        || value.pointer("/payload/id")?.as_str()? != parent_id
                    {
                        return None;
                    }
                    first_record = false;
                }
                if value.get("ordinal")?.as_u64()? != ordinal {
                    return None;
                }
                ordinal = ordinal.checked_add(1)?;
                if value.get("type").and_then(serde_json::Value::as_str) == Some("event_msg")
                    && value
                        .pointer("/payload/type")
                        .and_then(serde_json::Value::as_str)
                        == Some("token_count")
                {
                    if let Some(v) = value.pointer("/payload/info/total_token_usage") {
                        parent_total = parse_signature_counters(Some(v));
                    }
                }
            }
            if FileStamp::of(path)? != stamp {
                return None;
            }
            if used != end || ordinal != count || parent_total.as_ref() != Some(&baseline) {
                return None;
            }
            found = true;
        }
        found.then_some(())
    })()
    .is_some()
}

fn parsed_file(
    path: &Path,
    rollout_index: &RolloutIndex,
    cache: &mut CodexParseCache,
) -> Result<Arc<ParsedCodexFile>, String> {
    let stamp = FileStamp::of(path);
    if let (Some(stamp), Some((cached_stamp, parsed))) = (stamp, cache.parsed.get(path)) {
        if *cached_stamp == stamp {
            return Ok(Arc::clone(parsed));
        }
    }
    let parsed = Arc::new(parse_codex_file(
        path,
        thread_id_from_filename(path),
        rollout_index,
        &mut cache.timelines,
    )?);
    match stamp {
        Some(stamp) if !parsed.depends_on_other_files => {
            cache
                .parsed
                .insert(path.to_path_buf(), (stamp, Arc::clone(&parsed)));
        }
        _ => {
            cache.parsed.remove(path);
        }
    }
    Ok(parsed)
}

/// Read the Codex rollouts under `codex_home` and return the counted requests.
///
/// `skip` is asked for every rollout with its current stamp; returning true
/// leaves an unchanged, already recorded rollout unparsed. Skipped rollouts
/// still serve as parents of other rollouts.
pub fn scan_codex_usage(
    codex_home: &Path,
    cache: &mut CodexParseCache,
    skip: &dyn Fn(&Path, &FileStamp) -> bool,
) -> CodexScan {
    let files = collect_codex_session_files(codex_home);
    let rollout_index = build_rollout_index(&files);
    cache.retain_paths(&files.iter().collect());

    let mut scan = CodexScan {
        files_scanned: files.len(),
        ..CodexScan::default()
    };
    let mut seen_requests: HashSet<String> = HashSet::new();

    for path in &files {
        let stamp = FileStamp::of(path);
        if stamp.is_some_and(|stamp| skip(path, &stamp)) {
            continue;
        }
        let defer = |scan: &mut CodexScan, reason: String| {
            scan.deferred.push(DeferredFile {
                path: path.clone(),
                reason,
            });
            scan.files.push(ScannedFile {
                path: path.clone(),
                stamp,
                deferred: true,
                replayed_events: 0,
                duplicate_events: 0,
            });
        };
        let parsed = match parsed_file(path, &rollout_index, cache) {
            Ok(parsed) => parsed,
            Err(reason) => {
                defer(&mut scan, reason);
                continue;
            }
        };
        let mut scanned = ScannedFile {
            path: path.clone(),
            stamp,
            deferred: false,
            replayed_events: 0,
            duplicate_events: 0,
        };
        if !parsed.has_billable_tokens {
            scan.files.push(scanned);
            continue;
        }
        let Some(root_thread_id) = parsed.root_thread_id.as_deref() else {
            defer(&mut scan, "文件名缺少有效的尾部 UUID".into());
            continue;
        };
        if !parsed.root_meta_seen {
            defer(&mut scan, "含计费 token 但尚无 session_meta".into());
            continue;
        }

        let replay_prefix = match &parsed.parent {
            ParentResolution::None | ParentResolution::Continuation => 0,
            ParentResolution::Deferred(reason) => {
                defer(&mut scan, reason.clone());
                continue;
            }
            ParentResolution::Parent(_) if verified_bounded_fork(&parsed, &rollout_index) => 0,
            ParentResolution::Parent(parent_id) => {
                let Some(cutoff) = parsed.root_timestamp else {
                    defer(&mut scan, "子会话的 session_meta 缺少有效 timestamp".into());
                    continue;
                };
                match resolve_parent_signatures(
                    parent_id,
                    cutoff,
                    &rollout_index,
                    &mut cache.timelines,
                ) {
                    Ok(signatures) => matching_replay_prefix(&parsed.token_events, &signatures),
                    Err(reason) => {
                        defer(&mut scan, reason);
                        continue;
                    }
                }
            }
        };

        // Sessions are identified by the root meta thread ID (the leading UUID
        // for revert/resume file names); request keys stay per physical rollout.
        let session_thread_id = parsed.meta_thread_id.as_deref().unwrap_or(root_thread_id);
        let token_events = parsed
            .token_events
            .iter()
            .enumerate()
            .filter_map(|(offset, event)| {
                let index = event.event_index?;
                if offset < replay_prefix {
                    scanned.replayed_events += 1;
                    return None;
                }
                Some((event, index.to_string()))
            })
            .collect::<Vec<_>>();
        let record_events = parsed
            .usage_record_events
            .iter()
            .filter_map(|event| Some((event, event.request_suffix.clone()?)));
        for (event, suffix) in token_events.into_iter().chain(record_events) {
            let request_key = format!("{root_thread_id}:{suffix}");
            if !seen_requests.insert(request_key.clone()) {
                scanned.duplicate_events += 1;
                continue;
            }
            scan.events.push(CodexUsageEvent {
                request_key,
                thread_id: session_thread_id.to_string(),
                model: event.model.clone(),
                timestamp: parse_timestamp(
                    event
                        .timestamp
                        .as_ref()
                        .map(|value| serde_json::Value::String(value.clone()))
                        .as_ref(),
                ),
                tokens: event.delta,
            });
        }
        scan.replayed_events += scanned.replayed_events;
        scan.duplicate_events += scanned.duplicate_events;
        scan.files.push(scanned);
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    const PARENT_ID: &str = "00000000-0000-4000-8000-000000000001";
    const CHILD_ID: &str = "00000000-0000-4000-8000-000000000002";
    const PAGE_ID: &str = "00000000-0000-4000-8000-000000000004";

    fn write_jsonl(path: &Path, values: &[serde_json::Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let contents = values
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(path, contents).unwrap();
    }

    fn sessions_dir(home: &Path) -> PathBuf {
        home.join("sessions/2026/07/10")
    }

    fn rollout_path(home: &Path, thread_id: &str) -> PathBuf {
        sessions_dir(home).join(format!("rollout-2026-07-10T03-00-00-{thread_id}.jsonl"))
    }

    fn session_meta_at(thread_id: &str, spawned_from: Option<&str>, ts: &str) -> serde_json::Value {
        let source = spawned_from.map_or_else(
            || json!("cli"),
            |parent| json!({ "subagent": { "thread_spawn": { "parent_thread_id": parent } } }),
        );
        json!({
            "timestamp": ts,
            "type": "session_meta",
            "payload": { "id": thread_id, "source": source }
        })
    }

    fn session_meta(thread_id: &str) -> serde_json::Value {
        session_meta_at(thread_id, None, "2026-07-10T03:00:00Z")
    }

    fn turn_context(model: &str) -> serde_json::Value {
        json!({
            "timestamp": "2026-07-10T03:00:01Z",
            "type": "turn_context",
            "payload": { "model": model }
        })
    }

    fn token_count_at(input: u64, cached: u64, output: u64, ts: &str) -> serde_json::Value {
        json!({
            "timestamp": ts,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": { "total_token_usage": {
                    "input_tokens": input,
                    "cached_input_tokens": cached,
                    "output_tokens": output,
                    "reasoning_output_tokens": 0,
                    "total_tokens": input + output
                }}
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn token_count_with_last(
        total: (u64, u64, u64),
        last: (u64, u64, u64),
        limit_id: &str,
        ts: &str,
    ) -> serde_json::Value {
        let usage = |(input, cached, output): (u64, u64, u64)| {
            json!({
                "input_tokens": input,
                "cached_input_tokens": cached,
                "output_tokens": output,
                "reasoning_output_tokens": 0,
                "total_tokens": input + output
            })
        };
        json!({
            "timestamp": ts,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": { "total_token_usage": usage(total), "last_token_usage": usage(last) },
                "rate_limits": { "limit_id": limit_id }
            }
        })
    }

    fn usage_record(response_id: &str, input: u64, output: u64, ts: &str) -> serde_json::Value {
        json!({
            "timestamp": ts,
            "type": "token_usage_record",
            "payload": {
                "response_id": response_id,
                "usage": {
                    "input_tokens": input,
                    "cached_input_tokens": 0,
                    "output_tokens": output,
                    "total_tokens": input + output
                }
            }
        })
    }

    fn scan(home: &Path) -> CodexScan {
        scan_codex_usage(home, &mut CodexParseCache::default(), &|_, _| false)
    }

    fn summary(scan: &CodexScan) -> Vec<(String, u64, u64, u64)> {
        scan.events
            .iter()
            .map(|event| {
                (
                    event.model.clone(),
                    event.tokens.input,
                    event.tokens.cached_input,
                    event.tokens.output,
                )
            })
            .collect()
    }

    #[test]
    fn normalize_codex_model_rules() {
        assert_eq!(normalize_codex_model("GLM-4.6"), "glm-4.6");
        assert_eq!(normalize_codex_model("openai/gpt-5.4"), "gpt-5.4");
        assert_eq!(normalize_codex_model("gpt-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(normalize_codex_model("gpt-5.4-20260305"), "gpt-5.4");
        assert_eq!(normalize_codex_model("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(
            normalize_codex_model("Azure/GPT-5.4-Mini-2026-03-05"),
            "gpt-5.4-mini"
        );
        assert_eq!(normalize_codex_model("模型-2026-03-05"), "模型");
    }

    #[test]
    fn cumulative_snapshots_become_deltas() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), PARENT_ID),
            &[
                session_meta(PARENT_ID),
                turn_context("OpenAI/GPT-5.4-2026-03-05"),
                token_count_at(1_000, 800, 100, "2026-07-10T03:00:02Z"),
                token_count_at(1_500, 1_200, 160, "2026-07-10T03:00:03Z"),
            ],
        );
        let scan = scan(home.path());
        assert_eq!(
            summary(&scan),
            vec![
                ("gpt-5.4".into(), 1_000, 800, 100),
                ("gpt-5.4".into(), 500, 400, 60)
            ]
        );
        assert_eq!(scan.events[0].thread_id, PARENT_ID);
    }

    #[test]
    fn exact_last_usage_wins_and_cross_limit_replay_is_not_double_counted() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), PARENT_ID),
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_with_last(
                    (1_000, 0, 10),
                    (100, 0, 10),
                    "codex",
                    "2026-07-10T03:00:02Z",
                ),
                token_count_with_last(
                    (1_000, 0, 10),
                    (100, 0, 10),
                    "codex_other",
                    "2026-07-10T03:00:03Z",
                ),
            ],
        );
        assert_eq!(
            summary(&scan(home.path())),
            vec![("gpt-5.4".into(), 100, 0, 10)]
        );
    }

    #[test]
    fn thread_settings_switch_model_attribution() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), PARENT_ID),
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_at(100, 0, 10, "2026-07-10T03:00:02Z"),
                json!({
                    "timestamp": "2026-07-10T03:00:03Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "thread_settings_applied",
                        "thread_settings": { "model": "openai/GPT-5.5-20260601" }
                    }
                }),
                token_count_at(300, 0, 30, "2026-07-10T03:00:04Z"),
            ],
        );
        assert_eq!(
            summary(&scan(home.path())),
            vec![
                ("gpt-5.4".into(), 100, 0, 10),
                ("gpt-5.5".into(), 200, 0, 20)
            ]
        );
    }

    #[test]
    fn remote_compaction_usage_record_is_counted_once() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), PARENT_ID),
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                usage_record("resp_compact", 26_705, 491, "2026-07-10T03:00:02Z"),
                token_count_with_last((0, 0, 0), (0, 0, 0), "default", "2026-07-10T03:00:02Z"),
                usage_record("resp_turn", 1_000, 50, "2026-07-10T03:00:03Z"),
                token_count_with_last(
                    (1_000, 0, 50),
                    (1_000, 0, 50),
                    "default",
                    "2026-07-10T03:00:04Z",
                ),
            ],
        );
        assert_eq!(
            summary(&scan(home.path())),
            vec![
                ("gpt-5.4".into(), 1_000, 0, 50),
                ("gpt-5.4".into(), 26_705, 0, 491)
            ]
        );
    }

    #[test]
    fn sub_agent_replay_of_parent_history_is_skipped() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), PARENT_ID),
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_at(1_000, 900, 100, "2026-07-10T03:00:01Z"),
                turn_context("gpt-5.4"),
                json!({ "timestamp": "2026-07-10T03:00:10Z", "type": "turn_context", "payload": {} }),
            ],
        );
        write_jsonl(
            &rollout_path(home.path(), CHILD_ID),
            &[
                session_meta_at(CHILD_ID, Some(PARENT_ID), "2026-07-10T03:00:05Z"),
                turn_context("gpt-5.4-mini"),
                token_count_at(1_000, 900, 100, "2026-07-10T03:00:06Z"),
                token_count_at(1_300, 1_050, 150, "2026-07-10T03:00:07Z"),
            ],
        );
        let scan = scan(home.path());
        assert_eq!(
            summary(&scan),
            vec![
                ("gpt-5.4".into(), 1_000, 900, 100),
                ("gpt-5.4-mini".into(), 300, 150, 50)
            ]
        );
        assert_eq!(scan.replayed_events, 1);
        assert!(scan.deferred.is_empty());
    }

    #[test]
    fn child_without_parent_rollout_is_deferred() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), CHILD_ID),
            &[
                session_meta_at(CHILD_ID, Some(PARENT_ID), "2026-07-10T03:00:05Z"),
                token_count_at(1_000, 0, 100, "2026-07-10T03:00:06Z"),
            ],
        );
        let scan = scan(home.path());
        assert!(scan.events.is_empty());
        assert_eq!(scan.deferred.len(), 1);
        assert!(scan.deferred[0].reason.contains(PARENT_ID));
    }

    #[test]
    fn archived_copy_and_compressed_rollout_are_not_double_counted() {
        let home = tempdir().unwrap();
        let events = [
            session_meta(PARENT_ID),
            turn_context("gpt-5.4"),
            token_count_at(1_000, 0, 100, "2026-07-10T03:00:02Z"),
        ];
        write_jsonl(&rollout_path(home.path(), PARENT_ID), &events);
        let archived = home
            .path()
            .join("archived_sessions")
            .join(format!("rollout-2026-07-10T03-00-00-{PARENT_ID}.jsonl"));
        write_jsonl(&archived, &events);
        // A different, compressed rollout is read through zstd.
        let plain = events
            .iter()
            .map(|value| value.to_string().replace(PARENT_ID, CHILD_ID) + "\n")
            .collect::<String>();
        fs::write(
            sessions_dir(home.path())
                .join(format!("rollout-2026-07-10T03-00-00-{CHILD_ID}.jsonl.zst")),
            zstd::stream::encode_all(plain.as_bytes(), 3).unwrap(),
        )
        .unwrap();

        let scan = scan(home.path());
        assert_eq!(scan.files_scanned, 3);
        assert_eq!(scan.events.len(), 2);
        assert_eq!(scan.duplicate_events, 1);
    }

    #[test]
    fn continuation_page_uses_previous_segment_baseline() {
        let home = tempdir().unwrap();
        write_jsonl(
            &rollout_path(home.path(), PARENT_ID),
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_at(1_000, 0, 100, "2026-07-10T03:00:02Z"),
            ],
        );
        let page = sessions_dir(home.path()).join(format!(
            "rollout-2026-07-10T04-00-00-{PARENT_ID}_{PAGE_ID}.jsonl"
        ));
        write_jsonl(
            &page,
            &[
                json!({
                    "timestamp": "2026-07-10T04:00:00Z",
                    "type": "session_meta",
                    "payload": { "id": PARENT_ID, "history_mode": "paginated" }
                }),
                turn_context("gpt-5.4"),
                token_count_at(1_400, 0, 150, "2026-07-10T04:00:02Z"),
            ],
        );
        let mut cache = CodexParseCache::default();
        let scan = scan_codex_usage(home.path(), &mut cache, &|_, _| false);
        assert_eq!(
            summary(&scan),
            vec![
                ("gpt-5.4".into(), 1_000, 0, 100),
                ("gpt-5.4".into(), 400, 0, 50)
            ]
        );
        assert!(scan.events.iter().all(|event| event.thread_id == PARENT_ID));
        // The continuation result depends on another file and is not cached.
        assert!(!cache.parsed.contains_key(&page));
    }

    #[test]
    fn partial_final_record_is_ignored_until_complete() {
        let home = tempdir().unwrap();
        let path = rollout_path(home.path(), PARENT_ID);
        write_jsonl(
            &path,
            &[
                session_meta(PARENT_ID),
                token_count_at(100, 0, 10, "2026-07-10T03:00:02Z"),
            ],
        );
        let mut contents = fs::read_to_string(&path).unwrap();
        contents.push_str("{\"timestamp\":\"2026-07-10T03:00:03Z\",\"type\":\"event_msg\"");
        fs::write(&path, contents).unwrap();
        assert_eq!(scan(home.path()).events.len(), 1);
    }
}
