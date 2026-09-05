# Analog Logic agent guide

This file is the compact repository contract. Read it before editing. Keep it current when a public behavior, file boundary, or verification command changes.

## Goal

Maintain a small, readable Windows Tauri app that captures two Analog Discovery 3 analog channels, displays them as oscilloscope traces, and decodes protocols from sampled data.

## Source map

| Area | Read first |
| --- | --- |
| UI state and Tauri calls | `src/app/App.tsx` |
| Canvas rendering and navigation | `src/app/ScopeTrack.tsx` |
| Device access | `src-tauri/src/dwf.rs` |
| Acquisition, rolling data, envelopes, logs | `src-tauri/src/stream.rs` |
| Protocol bridge and annotation parsing | `src-tauri/src/analyzer.rs` |
| Tauri command registration | `src-tauri/src/main.rs` |

The normal data path is `dwf.rs → stream.rs → Tauri commands → App.tsx → ScopeTrack.tsx`. Keep device I/O in Rust and presentation logic in React.

## Contracts to preserve

- A1 and A2 are the only acquisition channels.
- Rust uses hertz, seconds, and volts consistently across command payloads and JSON files.
- `stream.rs` keeps a bounded rolling window but logs the full run until Stop.
- Lost or corrupt input must remain visible in integrity counters and must not be interpolated into the display.
- Tauri command names and serialized field names are part of the frontend/backend API.
- `dwf.dll` and `sigrok-cli` are installed locally; never commit them, captures, build output, or generated Tauri schemas.
- Hardware ownership is exclusive. Device start, stop, close, and window-close cleanup must stay safe on errors.

## Working rules

- Prefer the smallest change that makes the behavior clear. Avoid new files, wrappers, or abstractions unless they remove real duplication.
- Preserve existing behavior unless the task names a behavior change. Update `README.md` when user-visible behavior or setup changes.
- Before finalizing code, run `pnpm check` and inspect the diff. The AD3 integration test is ignored unless hardware is attached.
- Use the public README for user setup and behavior. Keep this file short and operational.
- Do not edit `dist/`, `node_modules/`, `src-tauri/target/`, `src-tauri/gen/`, `captures/`, `.tools/`, `.firecrawl/`, or `.local-notes/` as repository content.

## Astra operating profile

GPT-6 Astra should infer routine implementation details from this guide, complete authorized local work, and ask only when an unresolved choice would materially change the result. State assumptions and validation evidence briefly. Prefer direct prose, focused diffs, and meaningful checks over speculative refactors or duplicated tests.

Reference: [OpenAI model guidance for GPT-6 Astra](https://developers.openai.com/api/docs/guides/latest-model).
