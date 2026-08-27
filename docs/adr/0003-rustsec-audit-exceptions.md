# ADR 0003: RustSec audit exceptions for the MVP target graph

- Status: Accepted
- Date: 2026-08-27
- Review deadline: 2026-11-27
- Scope: T16 dependency governance gate

## Context

The T16 gate checks the macOS and Windows MVP target graph with `cargo-deny` and also runs the broader `cargo audit` lockfile report. On 2026-08-27 neither tool reported a known exploitable vulnerability in dbox's selected target graph. `cargo-deny` did report six transitive crates as unmaintained, so the gate requires an explicit, time-bounded exception instead of a command-line suppression.

`cargo audit` analyzes the whole lockfile and additionally reports GTK3/GLib advisories that are reachable only through Tauri's Linux dependency graph. Linux is not an MVP target. These entries remain visible in the audit report but are not added to the selected-target `cargo-deny` exception list.

## Decision

The repository's `deny.toml` is authoritative for the supported MVP targets: Apple Silicon macOS, Intel macOS, and x86-64 Windows MSVC. The following unmaintained-only advisories are temporarily accepted:

| Advisory | Dependency path | Impact analysis | Tracking action | Expiry |
| --- | --- | --- | --- | --- |
| `RUSTSEC-2024-0436` (`paste`) | `dbox → specta → paste` | The advisory states that the crate is unmaintained; it does not identify a vulnerability. dbox uses it transitively while deriving build-time API metadata. | Re-run the ADR 0002 compatibility spike and upgrade the pinned Specta family when a compatible maintained release removes `paste`. Track in T20 dependency review. | 2026-11-27 |
| `RUSTSEC-2025-0075`, `RUSTSEC-2025-0080`, `RUSTSEC-2025-0081`, `RUSTSEC-2025-0098`, `RUSTSEC-2025-0100` (`unic-*`) | `dbox → tauri → tauri-utils → urlpattern → unic-*` | These advisories state that the crates are unmaintained; they do not identify vulnerabilities. The code is not owned or called directly by dbox and is part of Tauri's URL-pattern implementation. | Upgrade Tauri when its supported dependency graph replaces `urlpattern`/`unic-*`; re-evaluate during T20 and before the expiry date. | 2026-11-27 |

The related `RUSTSEC-2025-0081` entry covers `unic-char-property`; all five configured `unic-*` exceptions share the same dependency path and remediation. Exceptions are permitted only for these exact advisory IDs. Newly reported vulnerabilities, yanked crates, unknown registries, wildcard direct requirements, or unknown licenses remain blocking.

## Review process

At T20, and no later than the review deadline:

1. Update the RustSec database and run both dependency gates again.
2. Test newer compatible Tauri and Specta releases on macOS and Windows.
3. Remove each exception whose dependency path is gone.
4. If an exception must continue, update this ADR with current impact evidence, a new bounded date, and a concrete tracking task; expiry must never be extended silently.

## Consequences

The backend gate remains strict for security advisories while making an explicit distinction between a known vulnerability and a maintenance-status warning. The accepted maintenance risk is visible in source control, limited to exact advisory IDs, and scheduled for review before MVP release.
