# ADR 0002: Rust-to-TypeScript API contract generation

- Status: Accepted
- Date: 2026-08-27
- Scope: T15 Tauri commands/events and T17 frontend transport
- Spike toolchain: Rust 1.97.1, Tauri 2.11.5, TypeScript 6.0.2

## Context

T15 defines stable public API DTOs, commands and events, while T17 consumes them from Vue. Handwritten TypeScript interfaces and invoke/event wrappers would create a second contract that can drift from serde wire formats. Type generation must not expose internal domain or persistence documents, and it must coexist with JSON golden tests.

The spike compared the intended primary path with narrower alternatives:

- `tauri-specta` generates command functions, typed error results, event listen/emit wrappers and referenced TypeScript types from the same Rust declarations.
- `specta` with a handwritten invoke layer and `ts-rs` generate types but leave command names, arguments, results and event wrappers to a second manually maintained layer.
- Handwritten TypeScript DTOs provide no mechanical drift protection and are rejected.

## Compatibility spike

A minimal command returning a public DTO and a typed run-output event was compiled with exact `tauri-specta = 2.0.0-rc.25`, `specta = 2.0.0-rc.25`, `specta-typescript = 0.0.12` and Tauri 2.11.5. It generated bindings that passed TypeScript 6.0.2 strict checking against the installed Tauri API.

The spike also verified an important failure mode: the exporter rejects Rust `u64` as a TypeScript `number` because JavaScript cannot represent every `u64` exactly. After changing the API event sequence to a decimal string, command and event bindings generated and type-checked successfully. This rejection is retained as a safety property; dbox will not enable the exporter's lossy BigInt-to-number option.

The selected Specta crates are MIT licensed. Their published metadata does not declare an MSRV, and the selected line remains a release candidate; the exact versions therefore must be pinned together and upgraded only through a repeat of this spike. The 2024-edition crates and their dependency set compile on the project's Rust 1.97.1 toolchain.

## Decision

1. T15 will add exact, mutually compatible `tauri-specta`, `specta` and `specta-typescript` versions with only the derive/TypeScript features needed for commands and events.
2. Only explicit public API DTOs derive `specta::Type`. Domain types, Provider output DTOs and persistence documents remain private and are converted at the command boundary.
3. Rust serde declarations are the single source for TypeScript DTO, command and event bindings. The generated file is committed, marked as generated and updated only through one repository command.
4. TypeScript code and mock transports import generated bindings/types. Handwritten mirror interfaces and duplicate command/event string registries are prohibited.
5. Potentially lossful 64-bit integers use decimal strings at the API boundary, unless a documented invariant permits a bounded type such as `u32`. The generated exporter must continue rejecting unreviewed wide integers.
6. CI regenerates bindings and fails on a repository diff, then runs the normal TypeScript build. Serde JSON golden tests remain mandatory because generation verifies type shape, not complete compatibility policy.
7. If the pinned release-candidate line becomes incompatible, update this ADR after comparing a newer `tauri-specta` release with `specta` or `ts-rs` plus a thin handwritten transport. Do not silently fall back to mirror DTOs.

## Consequences

The API layer gains release-candidate dependencies and requires deliberate coordinated upgrades. In return, Rust DTOs, command arguments/results and event payloads have one source, frontend mocks use the same types as production transport, and unsafe integer mappings fail during binding generation instead of losing precision at runtime.
