# Analog Logic

Analog Logic is a small Windows desktop oscilloscope for an Analog Discovery 3. It streams both analog inputs, renders min/max waveform envelopes, and decodes digital protocols over the same sampled signal.

The app uses React and Vite for the interface, Tauri 2 for the desktop shell, and Rust for device access, streaming, logging, and decoding.

## Requirements

- Windows with the Digilent WaveForms SDK installed so `dwf.dll` is available.
- Node.js with pnpm.
- Rust and Cargo.
- `sigrok-cli` for protocol decoding. Set `SIGROK_CLI` to its executable, or place it at `.tools/sigrok-cli/sigrok-cli.exe`.

The hardware SDK and decoder runtime are local dependencies. They are not included in this repository or the application bundle.

## Run

```powershell
pnpm install
pnpm tauri:dev
```

## Verify

```powershell
pnpm check
```

`pnpm check` runs the TypeScript/Vite build, Rust formatting check, and Rust unit tests. The hardware integration test is ignored by default:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml live_stream_start_stop_and_restart --ignored --nocapture -- --test-threads=1
```

Run it only with an attached Analog Discovery 3. The test exercises continuous acquisition, restart, bounded envelopes, stopped-state retention, and capture logging.

## Architecture

```text
Analog Discovery 3
        ↓
src-tauri/src/dwf.rs       WaveForms SDK loading and device I/O
        ↓
src-tauri/src/stream.rs    acquisition worker, rolling window, envelopes, logs
        ↓
Tauri commands
        ↓
src/app/App.tsx            UI state, controls, polling, decoder configuration
        ↓
src/app/ScopeTrack.tsx     canvas waveform, pan/zoom, crosshair, annotations
```

| File | Responsibility |
| --- | --- |
| `src/app/App.tsx` | Owns UI state and Tauri command calls. |
| `src/app/ScopeTrack.tsx` | Draws one channel and handles time/voltage navigation. |
| `src-tauri/src/dwf.rs` | Loads `dwf.dll`, configures AnalogIn, and reads samples. |
| `src-tauri/src/stream.rs` | Owns the acquisition worker, rolling samples, envelopes, and capture files. |
| `src-tauri/src/analyzer.rs` | Digitizes analog samples and runs the external decoder. |
| `src-tauri/src/main.rs` | Registers Tauri commands and stops acquisition on window close. |

## Data contracts

- There are two fixed channels: A1 and A2.
- Sample rate is in hertz, time is in seconds, and voltage is in volts.
- The UI receives min/max envelopes with at most 2,048 buckets per channel.
- The rolling display holds at most 2,000,000 samples per channel. The disk log continues until Stop.
- Lost or corrupt blocks create a new contiguous display segment; the renderer does not bridge gaps.
- Decoder results include their snapshot origin so annotations remain aligned with the correct capture window.
- Only one acquisition may own the AD3 at a time.

## Capture files

Each run writes `captures/stream-<timestamp>/`:

- `analog.f64le`: interleaved A1/A2 voltage samples as little-endian `f64` values.
- `chunks.jsonl`: sample positions, file offsets, and lost/corrupt counts.
- `metadata.json`: format, applied rate, ADC resolution, and display window.
- `summary.json`: final integrity counts after Stop.
- `last-window.csv`: the final contiguous display window.
- `decode-*/`: decoder settings, input bytes, annotations, and diagnostics.

Read [AGENTS.md](AGENTS.md) before changing code. It is the short, authoritative map for GPT-6 Astra and other coding agents.
