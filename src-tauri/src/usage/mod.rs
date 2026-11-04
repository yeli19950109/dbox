//! Local AI CLI usage statistics.
//!
//! Currently covers Codex: exact per-request token usage read from the JSONL
//! session logs under `$CODEX_HOME` (default `~/.codex`), with model names
//! normalized so dated / provider-prefixed variants aggregate together.
//! Read-only: dbox never writes to the Codex directory.

pub mod codex;
mod rollout_file;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::api::{ApiErrorCode, ApiErrorDto};
use codex::{scan_codex_usage, CodexParseCache, CodexScan, DeltaTokens};

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

pub struct UsageService {
    codex_home: PathBuf,
    cache: Mutex<CodexParseCache>,
}

impl UsageService {
    pub fn new(codex_home: PathBuf) -> Self {
        Self {
            codex_home,
            cache: Mutex::new(CodexParseCache::default()),
        }
    }

    /// `$CODEX_HOME`, or `~/.codex`.
    pub fn production() -> Self {
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
            .unwrap_or_else(|| PathBuf::from("/unavailable/.codex"));
        Self::new(codex_home)
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
        let scan = {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            scan_codex_usage(&self.codex_home, &mut cache)
        };
        Ok(build_report(
            &self.codex_home,
            scan,
            request.days,
            Utc::now(),
        ))
    }
}

fn local_date(timestamp: DateTime<Utc>) -> chrono::NaiveDate {
    timestamp.with_timezone(&Local).date_naive()
}

fn build_report(
    codex_home: &std::path::Path,
    scan: CodexScan,
    days: Option<u32>,
    now: DateTime<Utc>,
) -> CodexUsageReport {
    let first_day = days.map(|days| local_date(now) - chrono::Days::new(u64::from(days) - 1));
    let mut totals = TokenUsageDto::default();
    let mut models: BTreeMap<String, TokenUsageDto> = BTreeMap::new();
    let mut daily: BTreeMap<chrono::NaiveDate, TokenUsageDto> = BTreeMap::new();
    let mut sessions: HashSet<&str> = HashSet::new();

    for event in &scan.events {
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
        codex_home: codex_home.display().to_string(),
        available: codex_home.is_dir(),
        generated_at: now.to_rfc3339(),
        days,
        files_scanned: u32::try_from(scan.files_scanned).unwrap_or(u32::MAX),
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
        deferred: scan
            .deferred
            .into_iter()
            .map(|file| DeferredUsageFileDto {
                path: file.path.display().to_string(),
                reason: file.reason,
            })
            .collect(),
        replayed_requests: u32::try_from(scan.replayed_events).unwrap_or(u32::MAX),
        duplicate_requests: u32::try_from(scan.duplicate_events).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::codex::CodexUsageEvent;
    use super::*;

    fn event(
        thread: &str,
        model: &str,
        ts: Option<&str>,
        input: u64,
        output: u64,
    ) -> CodexUsageEvent {
        CodexUsageEvent {
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

    #[test]
    fn report_aggregates_models_days_and_sessions() {
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let scan = CodexScan {
            files_scanned: 2,
            events: vec![
                event("a", "gpt-5.4", Some("2026-07-10T11:00:00Z"), 100, 10),
                event("a", "gpt-5.5", Some("2026-07-10T11:30:00Z"), 1_000, 100),
                event("b", "gpt-5.4", Some("2026-06-01T11:00:00Z"), 200, 20),
                event("b", "gpt-5.4", None, 50, 5),
            ],
            ..CodexScan::default()
        };
        let report = build_report(std::path::Path::new("/missing"), scan, None, now);
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
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let scan = CodexScan {
            events: vec![
                event("a", "gpt-5.4", Some("2026-07-10T11:00:00Z"), 100, 10),
                event("b", "gpt-5.4", Some("2026-06-01T11:00:00Z"), 200, 20),
                event("c", "gpt-5.4", None, 50, 5),
            ],
            ..CodexScan::default()
        };
        let report = build_report(std::path::Path::new("/missing"), scan, Some(7), now);
        assert_eq!(report.totals.requests, 1);
        assert_eq!(report.sessions, 1);
    }

    #[test]
    fn invalid_day_window_is_rejected() {
        let service = UsageService::new(PathBuf::from("/missing"));
        let error = service
            .codex_usage(CodexUsageRequest { days: Some(0) })
            .unwrap_err();
        assert_eq!(error.code, ApiErrorCode::InvalidRequest);
        let report = service
            .codex_usage(CodexUsageRequest { days: Some(30) })
            .unwrap();
        assert_eq!(report.files_scanned, 0);
    }
}
