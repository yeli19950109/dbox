# ADR 0001: T07–T09 Rust infrastructure dependencies

- Status: Accepted
- Date: 2026-08-27
- Scope: command plans, command execution, persistence support, and GUI environment resolution
- Project toolchain used for the spike: Rust 1.97.1, Tauri 2.11, Tokio 1.x

## Context

dbox needs stable identifiers and hashes for confirmed plans, safe secret handling, display-only shell quoting, bounded asynchronous process execution, process-tree cleanup, cancellation, ANSI/non-UTF-8 handling, and cross-platform executable discovery. These are infrastructure concerns and must not become project-specific reimplementations.

## Decision

The exact resolved releases are recorded by `src-tauri/Cargo.lock`. Direct requirements intentionally stay within the compatible lines below.

| Concern | Decision | License / MSRV | Maintenance and compatibility notes |
| --- | --- | --- | --- |
| DTOs and process command | `serde`, `serde_json`, `std::process::Command`, `tokio::process::Command` | MIT/Apache-2.0; follows the locked versions | Already used by Tauri and Tokio; dbox only maps domain fields to argv. |
| Plan IDs | `uuid` 1.x with v4/serde | MIT/Apache-2.0 | Widely maintained; native and WASM support. |
| Plan hashes and file etags | official `blake3` 1.8.x | CC0-1.0 or Apache-2.0 variants | Official implementation, active 2026 releases, SIMD support on desktop platforms. |
| Secrets | `secrecy` 0.10.x | MIT/Apache-2.0 | Secret values stay in `SecretString`; only explicit `ExposeSecret` calls reach hash/execution adapters. Serialized/display DTOs are redacted. |
| Display quoting | `shell-quote` 0.7.2 using Bash syntax | Apache-2.0; upstream does not declare an MSRV | Supports strings, bytes, paths and public vectors for empty/space/Unicode/quote/control inputs on macOS/Linux/Windows builds. Output is display-only and is never parsed for execution. `shlex` was not selected because `shell-quote` explicitly supports multiple shell dialects and byte/path inputs. |
| Validation | Small dbox cross-field checks | Project MSRV | `garde`/`validator` were evaluated but rejected here: filesystem existence, absolute-path, nested post-check, snapshot and expiry rules are cross-field/runtime checks, while field syntax is small. No general validation framework is implemented. |
| Async process and I/O | `tokio` 1.x | MIT; locked release supports the project toolchain | Shared runtime with Tauri; provides process, bounded channels, async I/O and timeout/select. |
| Process groups and cleanup | `process-wrap` 10.0.0 with `tokio1` and default wrappers | MIT/Apache-2.0; MSRV 1.87 | Active successor to `command-group`. POSIX process groups, Windows Job Objects, and Tokio kill-on-drop are supplied by the crate. dbox contains no signals, PID-tree walking, platform `unsafe`, or process-group implementation. |
| Cancellation | `tokio-util` 0.7.x `CancellationToken` | MIT; MSRV 1.71 | Maintained with Tokio; cancellation is combined with `tokio::select!` and process-wrap cleanup. |
| Output bytes and ANSI | `bytes`, `bstr`, `strip-ansi-escapes` | MIT/Apache-2.0 family | Bounded chunks and lossy UTF-8 handling avoid panics; ANSI removal is delegated to the crate. |
| Internal diagnostics | `tracing` 0.1.x | MIT | dbox emits structured internal spans/events. Run-specific JSONL remains owned by T06 persistence so revision and retention policy have one owner; `tracing-appender` is therefore not used as a second run-log writer. |
| Atomic document replacement | `atomic-write-file` exactly 0.3.1 with no optional features | BSD-3-Clause; MSRV 1.85 | Verified with Rust 1.97.1 on macOS. The crate owns same-directory temporary files, file sync, replacement, discard cleanup, Unix mode/owner preservation attempts, and Unix parent-directory `fsync`. dbox only creates parent directories and maps errors. `tempfile::NamedTempFile::persist` was not selected because it would still require dbox to add file and parent-directory durability behavior. |
| GUI PATH repair | Tauri `fix-path-env-rs` pinned to `c4c45d503ea115a839aae718d02f79e7c7f0f673` | MIT/Apache-2.0; upstream declares no MSRV | Official Tauri repository. It is called once at the controlled desktop entry and isolated behind an adapter. Failure is diagnostic and does not remove the inherited PATH or explicit user overrides. |
| Executable discovery | `which` 8.x (`which_in_all`) | MIT; MSRV 1.70 | Active cross-platform crate. dbox supplies the PATH input and only ranks returned absolute candidates; it does not split PATH, inspect executable permission bits, or parse shell profiles. |
| Application directories | Tauri 2 `PathResolver` | MIT/Apache-2.0; same as the app | Production directories are injected from `app_config_dir`, `app_data_dir`, and `app_log_dir`; tests inject temporary directories. |

Primary references:

- <https://github.com/watchexec/process-wrap>
- <https://docs.rs/process-wrap/10.0.0/process_wrap/tokio/>
- <https://docs.rs/shell-quote/0.7.2/shell_quote/>
- <https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html>
- <https://github.com/tauri-apps/fix-path-env-rs>
- <https://docs.rs/which/8/which/>
- <https://docs.rs/secrecy/0.10/secrecy/>
- <https://docs.rs/blake3/1/blake3/>
- <https://docs.rs/atomic-write-file/0.3.1/atomic_write_file/>

## Failure behavior

- Plan construction fails before confirmation for relative/missing programs, invalid working directories, NUL values, nested post-checks, or empty success-code sets.
- A changed configuration revision, environment revision, version snapshot, plan ID/hash, or expiry rejects confirmation.
- Spawn/I/O/wait errors map to stable dbox error kinds. Timeout, cancellation, and drop all rely on process-wrap cleanup.
- PATH repair failures are preserved in diagnostics. Resolution continues with user override, cache, inherited PATH candidates, and provider paths in that order.
- `which` errors become unavailable diagnostics; no fallback shell/profile parser is introduced.
- Atomic-write open/write/commit failures map to the destination path. Dropping or explicitly discarding an uncommitted write preserves the previous target; Unix commit also syncs the containing directory. The crate does not promise cross-platform preservation of ownership, ACLs, xattrs or timestamps, so dbox does not make those part of its wire/storage contract.

## Consequences

The dependency graph is larger, but sensitive infrastructure stays in maintained crates. dbox code is limited to domain validation, priority/retention policy, event sequencing, redaction policy, and mapping third-party results into stable DTOs.
