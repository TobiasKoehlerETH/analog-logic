# Analog Logic

Analog Logic is a small Windows desktop oscilloscope for an Analog Discovery 3. It streams both analog inputs, renders min/max waveform envelopes, and decodes digital protocols over the same sampled signal.

The app uses React and Vite for the interface, uPlot for waveforms, Tauri 2 for the desktop shell, and Rust for device access, streaming, logging, and decoding. Each chart reuses its uPlot instance and renders at most 2,048 min/max buckets per update, preserving brief pulses without sending the full capture to the UI.

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
cargo test --manifest-path src-tauri/Cargo.toml live_stream_start_stop_and_restart -- --ignored --nocapture --test-threads=1
```

Run it only with an attached Analog Discovery 3. The test exercises continuous acquisition, restart, bounded envelopes, stopped-state retention, and capture logging.

### Simulated waveform regression

Run `pnpm check:scope` with Microsoft Edge and the local `sigrok-cli` installed. No hardware is used. The test starts Vite if needed, generates a valid 500 kbit/s CAN frame (ID `0x123`, eight data bytes), and verifies sigrok decoding without warnings. It feeds simulated A1/CAN-L and A2/CAN-H voltage envelopes into the real UI and checks painted waveform pixels, repeated updates, zoom, resize, stopped-state retention, and annotation alignment at 100% and 200% display scaling. Screenshots and the synthetic decoder input are saved in a reported temporary directory.

For live differential CAN decoding, connect CAN-L to A1 and CAN-H to A2, use DC coupling, select CAN on A1, and enable the differential option in A1 decoder settings. This uses `A2 − A1` with a 0.9 V differential threshold and disables the separate A2 decoder. Both analog traces remain visible. Set the bitrate to match the bus.

The custom **IMU firmware** protocol builds on CAN decoding. Selecting it sets the bitrate to 1,000,000 bit/s and reassembles `0x100` (signed big-endian `ax`, `ay`, `az` in mg), `0x101` (signed big-endian `gx`, `gy` in mdps), and `0x102` (signed big-endian `gz` in mdps) into complete IMU samples. Non-IMU CAN frames are ignored; generic CAN remains available separately. Use at least 10 MS/s for a 1 Mbit/s bus; 20 MS/s is recommended.

The **Decode as** selector reformats existing byte overlays immediately, including after Stop; it does not recapture or rerun the decoder. CAN payload bytes use compact `D0`, `D1`, … labels in Hex, ASCII, Decimal, or Binary. Frame identifiers, CRCs, and warnings retain their descriptive text.

The default view gesture is **Window zoom**: drag across a trace to select a time window. Choose **Pan** from the View menu when dragging should move the current window instead.

Each chart renders one voltage line. The acquisition layer still retains min/max samples per display bucket so short pulses are preserved when zooming or decoding.

During a live lost/corrupt-sample gap, the UI holds the last valid waveform on screen while the footer counters continue to report the current integrity state; it does not interpolate across the gap.

Decoded boxes appear in a dedicated annotation rail above the waveform, only when their visible time interval is wide enough for the full label. They never cover waveform pixels; hovering or focusing a box lightly highlights its decoded time interval on the trace. Zoom in to reveal more labels. This check also follows the selected display format, with at most 200 readable boxes per channel.

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
src/app/ScopeTrack.tsx     uPlot waveform, pan/zoom, crosshair, annotations
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
- Start runs continuously until Stop. Choose the live time window before starting; the display keeps that span using bounded min/max buckets, including at 100 MS/s. The decoder retains at most 2,000,000 raw samples per channel, while the disk log continues for the full run.
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
