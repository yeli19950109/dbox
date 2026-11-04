//! Local AI CLI usage statistics.
//!
//! Currently covers Codex: exact per-request token usage read from the JSONL
//! session logs under `$CODEX_HOME` (default `~/.codex`), with model names
//! normalized so dated / provider-prefixed variants aggregate together.
//! dbox never writes to the Codex directory. Counted requests are kept in a
//! JSONL ledger under dbox's data directory (see [`ledger`]), so only new or
//! changed session files are parsed and usage survives deleted session logs.

pub mod codex;
pub mod ledger;
mod rollout_file;

use std::collections::{BTreeMap, HashSet};
#[cfg(test)]
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::api::{ApiErrorCode, ApiErrorDto};
use codex::{scan_codex_usage, CodexParseCache, CodexUsageEvent, DeferredFile, DeltaTokens};
use ledger::CodexLedger;

const MAX_DAYS: u32 = 3650;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageRequest {
    /// Only count requests from the last N local days (today included); all
    /// history when omitted.
    #[serde(default)]
    pub days: Option<u32>,
}

/// Token totals. Counts are JSON numbers (exact up to 2^53).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageDto {
    pub requests: u32,
    /// Total input tokens, including cached input.
    pub input_tokens: f64,
    pub cached_input_tokens: f64,
    pub cache_write_input_tokens: f64,
    pub output_tokens: f64,
    pub total_tokens: f64,
}

impl TokenUsageDto {
    fn add(&mut self, tokens: &DeltaTokens) {
        self.requests = self.requests.saturating_add(1);
        self.input_tokens += tokens.input as f64;
        self.cached_input_tokens += tokens.cached_input as f64;
        self.cache_write_input_tokens += tokens.cache_write_input as f64;
        self.output_tokens += tokens.output as f64;
        self.total_tokens += (tokens.input + tokens.output) as f64;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageDto {
    pub model: String,
    pub usage: TokenUsageDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsageDto {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub usage: TokenUsageDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeferredUsageFileDto {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageReport {
    pub codex_home: String,
    /// Directory holding dbox's usage ledger.
    pub ledger_dir: String,
    /// Whether `codex_home` exists.
    pub available: bool,
    pub generated_at: String,
    pub days: Option<u32>,
    pub files_scanned: u32,
    pub sessions: u32,
    pub totals: TokenUsageDto,
    /// Sorted by total tokens, descending.
    pub models: Vec<ModelUsageDto>,
    /// Sorted by date, ascending.
    pub daily: Vec<DailyUsageDto>,
    /// Rollouts whose usage could not be attributed reliably and is excluded.
    pub deferred: Vec<DeferredUsageFileDto>,
    /// Sub-agent / fork requests skipped because they replay parent history.
    pub replayed_requests: u32,
    /// Requests skipped because another copy of the rollout was counted.
    pub duplicate_requests: u32,
}

#[derive(Default)]
struct UsageState {
    cache: CodexParseCache,
    /// Loaded on first use.
    ledger: Option<CodexLedger>,
}

pub struct UsageService {
    codex_home: PathBuf,
    ledger_dir: PathBuf,
    state: Mutex<UsageState>,
}

impl UsageService {
    pub fn new(codex_home: PathBuf, ledger_dir: PathBuf) -> Self {
        Self {
            codex_home,
            ledger_dir,
            state: Mutex::new(UsageState::default()),
        }
    }

    /// Reads `$CODEX_HOME` (or `~/.codex`) and keeps the ledger in `ledger_dir`.
    pub fn production(ledger_dir: PathBuf) -> Self {
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
            .unwrap_or_else(|| PathBuf::from("/unavailable/.codex"));
        Self::new(codex_home, ledger_dir)
    }

    pub fn codex_usage(&self, request: CodexUsageRequest) -> Result<CodexUsageReport, ApiErrorDto> {
        if let Some(days) = request.days {
            if days == 0 || days > MAX_DAYS {
                return Err(ApiErrorDto::new(
                    ApiErrorCode::InvalidRequest,
                    format!("统计天数必须在 1 到 {MAX_DAYS} 之间"),
                    false,
                )
                .with_detail("field", "days"));
            }
        }
        let ledger_error = |action: &str, error: std::io::Error| {
            ApiErrorDto::new(
                ApiErrorCode::Unavailable,
                format!("{action}用量记录失败: {error}"),
                true,
            )
            .with_detail("path", self.ledger_dir.display().to_string())
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let UsageState { cache, ledger } = &mut *state;
        let ledger = match ledger {
            Some(ledger) => ledger,
            None => ledger.insert(
                CodexLedger::open(&self.ledger_dir).map_err(|error| ledger_error("读取", error))?,
            ),
        };
        let scan = scan_codex_usage(&self.codex_home, cache, &|path, stamp| {
            ledger.is_current(path, stamp)
        });
        ledger
            .record(&scan)
            .map_err(|error| ledger_error("写入", error))?;
        Ok(build_report(
            ReportInput {
                codex_home: &self.codex_home,
                ledger_dir: &self.ledger_dir,
                events: ledger.events(),
                files_scanned: scan.files_scanned,
                deferred: scan.deferred,
                replayed_events: ledger.replayed_events(),
                duplicate_events: ledger.duplicate_events(),
            },
            request.days,
            Utc::now(),
        ))
    }
}

fn local_date(timestamp: DateTime<Utc>) -> chrono::NaiveDate {
    timestamp.with_timezone(&Local).date_naive()
}

struct ReportInput<'a> {
    codex_home: &'a std::path::Path,
    ledger_dir: &'a std::path::Path,
    events: &'a [CodexUsageEvent],
    files_scanned: usize,
    deferred: Vec<DeferredFile>,
    replayed_events: u64,
    duplicate_events: u64,
}

fn build_report(input: ReportInput<'_>, days: Option<u32>, now: DateTime<Utc>) -> CodexUsageReport {
    let first_day = days.map(|days| local_date(now) - chrono::Days::new(u64::from(days) - 1));
    let mut totals = TokenUsageDto::default();
    let mut models: BTreeMap<String, TokenUsageDto> = BTreeMap::new();
    let mut daily: BTreeMap<chrono::NaiveDate, TokenUsageDto> = BTreeMap::new();
    let mut sessions: HashSet<&str> = HashSet::new();

    for event in input.events {
        let date = event.timestamp.map(local_date);
        if let Some(first_day) = first_day {
            // Requests without a timestamp cannot be placed in a window.
            if date.is_none_or(|date| date < first_day) {
                continue;
            }
        }
        totals.add(&event.tokens);
        models
            .entry(event.model.clone())
            .or_default()
            .add(&event.tokens);
        if let Some(date) = date {
            daily.entry(date).or_default().add(&event.tokens);
        }
        sessions.insert(event.thread_id.as_str());
    }

    let mut models = models
        .into_iter()
        .map(|(model, usage)| ModelUsageDto { model, usage })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        right
            .usage
            .total_tokens
            .total_cmp(&left.usage.total_tokens)
            .then_with(|| left.model.cmp(&right.model))
    });

    CodexUsageReport {
        codex_home: input.codex_home.display().to_string(),
        ledger_dir: input.ledger_dir.display().to_string(),
        available: input.codex_home.is_dir(),
        generated_at: now.to_rfc3339(),
        days,
        files_scanned: u32::try_from(input.files_scanned).unwrap_or(u32::MAX),
        sessions: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
        totals,
        models,
        daily: daily
            .into_iter()
            .map(|(date, usage)| DailyUsageDto {
                date: date.format("%Y-%m-%d").to_string(),
                usage,
            })
            .collect(),
        deferred: input
            .deferred
            .into_iter()
            .map(|file| DeferredUsageFileDto {
                path: file.path.display().to_string(),
                reason: file.reason,
            })
            .collect(),
        replayed_requests: u32::try_from(input.replayed_events).unwrap_or(u32::MAX),
        duplicate_requests: u32::try_from(input.duplicate_events).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn event(
        thread: &str,
        model: &str,
        ts: Option<&str>,
        input: u64,
        output: u64,
    ) -> CodexUsageEvent {
        CodexUsageEvent {
            request_key: format!("{thread}:{ts:?}:{input}"),
            thread_id: thread.into(),
            model: model.into(),
            timestamp: ts.map(|ts| {
                DateTime::parse_from_rfc3339(ts)
                    .unwrap()
                    .with_timezone(&Utc)
            }),
            tokens: DeltaTokens {
                input,
                cached_input: input / 2,
                cache_write_input: 0,
                output,
            },
        }
    }

    fn input(events: &[CodexUsageEvent]) -> ReportInput<'_> {
        ReportInput {
            codex_home: Path::new("/missing"),
            ledger_dir: Path::new("/ledger"),
            events,
            files_scanned: 2,
            deferred: Vec::new(),
            replayed_events: 0,
            duplicate_events: 0,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn report_aggregates_models_days_and_sessions() {
        let events = vec![
            event("a", "gpt-5.4", Some("2026-07-10T11:00:00Z"), 100, 10),
            event("a", "gpt-5.5", Some("2026-07-10T11:30:00Z"), 1_000, 100),
            event("b", "gpt-5.4", Some("2026-06-01T11:00:00Z"), 200, 20),
            event("b", "gpt-5.4", None, 50, 5),
        ];
        let report = build_report(input(&events), None, now());
        assert!(!report.available);
        assert_eq!(report.sessions, 2);
        assert_eq!(report.totals.requests, 4);
        assert_eq!(report.totals.total_tokens, 1_485.0);
        assert_eq!(report.totals.cached_input_tokens, 675.0);
        assert_eq!(
            report
                .models
                .iter()
                .map(|m| (m.model.as_str(), m.usage.requests))
                .collect::<Vec<_>>(),
            vec![("gpt-5.5", 1), ("gpt-5.4", 3)]
        );
        assert_eq!(report.daily.len(), 2);
        assert_eq!(
            report.daily.iter().map(|d| d.usage.requests).sum::<u32>(),
            3
        );
    }

    #[test]
    fn report_window_excludes_old_and_untimed_requests() {
        let events = vec![
            event("a", "gpt-5.4", Some("2026-07-10T11:00:00Z"), 100, 10),
            event("b", "gpt-5.4", Some("2026-06-01T11:00:00Z"), 200, 20),
            event("c", "gpt-5.4", None, 50, 5),
        ];
        let report = build_report(input(&events), Some(7), now());
        assert_eq!(report.totals.requests, 1);
        assert_eq!(report.sessions, 1);
    }

    #[test]
    fn invalid_day_window_is_rejected() {
        let ledger = tempdir().unwrap();
        let service = UsageService::new(PathBuf::from("/missing"), ledger.path().to_path_buf());
        let error = service
            .codex_usage(CodexUsageRequest { days: Some(0) })
            .unwrap_err();
        assert_eq!(error.code, ApiErrorCode::InvalidRequest);
        let report = service
            .codex_usage(CodexUsageRequest { days: Some(30) })
            .unwrap();
        assert_eq!(report.files_scanned, 0);
    }

    const THREAD_ID: &str = "00000000-0000-4000-8000-000000000001";

    fn token_line(total_input: u64, input: u64, ts: &str) -> String {
        let usage = |value: u64| {
            serde_json::json!({
                "input_tokens": value, "cached_input_tokens": 0,
                "output_tokens": 10, "total_tokens": value + 10
            })
        };
        serde_json::json!({
            "timestamp": ts,
            "type": "event_msg",
            "payload": { "type": "token_count", "info": {
                "total_token_usage": usage(total_input),
                "last_token_usage": usage(input)
            }}
        })
        .to_string()
            + "\n"
    }

    fn total_input(service: &UsageService) -> f64 {
        service
            .codex_usage(CodexUsageRequest::default())
            .unwrap()
            .totals
            .input_tokens
    }

    #[test]
    fn ledger_keeps_usage_across_restarts_appends_and_deleted_logs() {
        let codex = tempdir().unwrap();
        let ledger = tempdir().unwrap();
        let rollout = codex.path().join(format!(
            "sessions/2026/07/10/rollout-2026-07-10T03-00-00-{THREAD_ID}.jsonl"
        ));
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        let meta = serde_json::json!({
            "timestamp": "2026-07-10T03:00:00Z",
            "type": "session_meta",
            "payload": { "id": THREAD_ID }
        })
        .to_string()
            + "\n";
        fs::write(
            &rollout,
            meta.clone() + &token_line(100, 100, "2026-07-10T03:00:01Z"),
        )
        .unwrap();

        let service = UsageService::new(codex.path().to_path_buf(), ledger.path().to_path_buf());
        assert_eq!(total_input(&service), 100.0);
        assert_eq!(total_input(&service), 100.0);

        // Appended usage is counted once, also by a fresh process.
        fs::write(
            &rollout,
            meta + &token_line(100, 100, "2026-07-10T03:00:01Z")
                + &token_line(350, 250, "2026-07-10T03:00:02Z"),
        )
        .unwrap();
        assert_eq!(total_input(&service), 350.0);
        let restarted = UsageService::new(codex.path().to_path_buf(), ledger.path().to_path_buf());
        assert_eq!(total_input(&restarted), 350.0);

        // Usage survives deleted session logs.
        fs::remove_file(&rollout).unwrap();
        let restarted = UsageService::new(codex.path().to_path_buf(), ledger.path().to_path_buf());
        let report = restarted.codex_usage(CodexUsageRequest::default()).unwrap();
        assert_eq!(report.totals.input_tokens, 350.0);
        assert_eq!(report.files_scanned, 0);
        let lines = fs::read_to_string(ledger.path().join(ledger::REQUESTS_FILE)).unwrap();
        assert_eq!(lines.lines().count(), 2);
    }
}
