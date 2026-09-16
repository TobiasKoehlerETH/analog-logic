use crate::{
    analyzer::{self, DecodeConfig, DecodeResult, LoggedCapture, SignalStats},
    dwf::{self, AnalogConfig},
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
const MAX_RAW_WINDOW_SAMPLES: usize = 2_000_000;
const MAX_DISPLAY_BUCKETS: usize = 262_144;

struct Control {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
}
static CONTROL: OnceLock<Mutex<Option<Control>>> = OnceLock::new();
fn control() -> &'static Mutex<Option<Control>> {
    CONTROL.get_or_init(|| Mutex::new(None))
}
struct State {
    running: bool,
    directory: String,
    rate: f64,
    bits: i32,
    total: u64,
    received: u64,
    lost: u64,
    corrupt: u64,
    channels: [VecDeque<f64>; 2],
    capacity: usize,
    window_samples: u64,
    display_bucket_size: usize,
    display_capacity: usize,
    display: VecDeque<DisplayBucket>,
    error: Option<String>,
    sequence: u64,
}

#[derive(Clone)]
struct DisplayBucket {
    sample_start: u64,
    count: usize,
    min: [f64; 2],
    max: [f64; 2],
}
#[derive(Clone, Serialize)]
pub struct Envelope {
    pub start_s: f64,
    pub step_s: f64,
    pub min: Vec<f64>,
    pub max: Vec<f64>,
}
#[derive(Serialize)]
pub struct StreamView {
    running: bool,
    directory: String,
    sample_rate_hz: f64,
    adc_bits: i32,
    total_samples: u64,
    samples_lost: u64,
    samples_corrupt: u64,
    sample_count: usize,
    origin_s: f64,
    duration_s: f64,
    sequence: u64,
    error: Option<String>,
    envelopes: [Envelope; 2],
    stats: Vec<SignalStats>,
}
#[derive(Serialize)]
pub struct StreamDecoded {
    pub origin_s: f64,
    pub sequence: u64,
    pub results: Vec<Result<DecodeResult, String>>,
}
fn shared() -> Result<Arc<Mutex<State>>, String> {
    control()
        .lock()
        .map_err(err)?
        .as_ref()
        .map(|c| c.state.clone())
        .ok_or("Start the AD3 stream first".into())
}

fn window_sample_count(rate: f64, window_ms: f64) -> Result<u64, String> {
    let count = rate * window_ms / 1000.0;
    if !rate.is_finite()
        || rate <= 0.0
        || !window_ms.is_finite()
        || window_ms <= 0.0
        || !count.is_finite()
        || count < 8.0
        || count > u64::MAX as f64
    {
        return Err(
            "Choose a positive rate and display window containing at least 8 samples.".into(),
        );
    }
    Ok(count.ceil() as u64)
}

fn display_config(window_samples: u64) -> (usize, usize) {
    let bucket_size = window_samples.div_ceil(MAX_DISPLAY_BUCKETS as u64).max(1) as usize;
    let capacity = window_samples
        .div_ceil(bucket_size as u64)
        .min(MAX_DISPLAY_BUCKETS as u64) as usize;
    (bucket_size, capacity.max(1))
}

#[tauri::command]
pub async fn start_stream(index: i32, sample_rate_hz: f64, window_ms: f64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || start(index, sample_rate_hz, window_ms))
        .await
        .map_err(err)?
}
fn start(index: i32, rate: f64, window_ms: f64) -> Result<(), String> {
    let window_samples = window_sample_count(rate, window_ms)?;
    let (display_bucket_size, display_capacity) = display_config(window_samples);
    let mut manager = control().lock().map_err(err)?;
    if let Some(existing) = manager.as_ref() {
        if existing.worker.as_ref().is_some_and(|w| !w.is_finished()) {
            return Err("The stream is already running".into());
        }
    }
    if let Some(mut previous) = manager.take() {
        if let Some(worker) = previous.worker.take() {
            let _ = worker.join();
        }
    }
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../captures")
        .join(format!(
            "stream-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
    fs::create_dir_all(&directory).map_err(err)?;
    let state = Arc::new(Mutex::new(State {
        running: true,
        directory: directory.to_string_lossy().into_owned(),
        rate,
        bits: 0,
        total: 0,
        received: 0,
        lost: 0,
        corrupt: 0,
        channels: [VecDeque::new(), VecDeque::new()],
        capacity: window_samples.min(MAX_RAW_WINDOW_SAMPLES as u64) as usize,
        window_samples,
        display_bucket_size,
        display_capacity,
        display: VecDeque::new(),
        error: None,
        sequence: 0,
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let task_state = state.clone();
    let task_stop = stop.clone();
    let worker = thread::spawn(move || {
        let result = run(index, rate, window_ms, &task_state, &task_stop, &ready_tx);
        if let Ok(mut state) = task_state.lock() {
            state.running = false;
            if let Err(e) = result {
                let _ = ready_tx.try_send(Err(e.clone()));
                state.error = Some(e);
            }
        }
    });
    *manager = Some(Control {
        stop,
        state,
        worker: Some(worker),
    });
    drop(manager);
    ready_rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(err)?
}
fn run(
    index: i32,
    rate: f64,
    window_ms: f64,
    state: &Arc<Mutex<State>>,
    stop: &AtomicBool,
    ready: &std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let _owner = analyzer::ACQUISITION
        .try_lock()
        .map_err(|_| "AD3 acquisition is already in use")?;
    let caps = dwf::open_ad3(index, -1)?;
    let result = (|| {
        let applied = dwf::configure_ad3_analog(AnalogConfig {
            channels: vec![0, 1],
            frequency_hz: rate,
            buffer_size: caps.analog_buffer_max,
            range_v: 10.0,
            offset_v: 0.0,
            acquisition_mode: 3,
            record_length_s: 0.0,
            filter: 0,
            trigger: None,
            coupling: None,
            bandwidth_hz: None,
            attenuation: None,
            impedance_ohm: None,
        })?;
        let directory = {
            let mut s = state.lock().map_err(err)?;
            s.rate = applied.frequency_hz;
            s.bits = caps.analog_bits;
            s.window_samples = window_sample_count(applied.frequency_hz, window_ms)?;
            s.capacity = s.window_samples.min(MAX_RAW_WINDOW_SAMPLES as u64) as usize;
            (s.display_bucket_size, s.display_capacity) = display_config(s.window_samples);
            s.display.clear();
            PathBuf::from(&s.directory)
        };
        let mut data_log = BufWriter::with_capacity(
            1024 * 1024,
            File::create(directory.join("analog.f64le")).map_err(err)?,
        );
        let mut chunks = BufWriter::new(File::create(directory.join("chunks.jsonl")).map_err(err)?);
        fs::write(directory.join("metadata.json"), serde_json::to_vec_pretty(&serde_json::json!({"schema_version":2,"source":"AD3 continuous AnalogIn record","sample_rate_hz":applied.frequency_hz,"adc_bits":caps.analog_bits,"format":"little-endian f64, interleaved A1,A2 volts; chunks.jsonl preserves sample positions and gaps","window_ms":window_ms})).map_err(err)?).map_err(err)?;
        // Settling is cancellable; the hardware is only armed once.
        for _ in 0..20 {
            if stop.load(Ordering::Relaxed) {
                let _ = ready.try_send(Ok(()));
                return Ok(());
            }
            thread::sleep(Duration::from_millis(5));
        }
        dwf::set_analog_stream_running(true)?;
        let _ = ready.send(Ok(()));
        let mut last_data = Instant::now();
        let mut last_flush = Instant::now();
        let mut read_buffers = [
            Vec::with_capacity(caps.analog_buffer_max.max(0) as usize),
            Vec::with_capacity(caps.analog_buffer_max.max(0) as usize),
        ];
        let mut packed = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let chunk = dwf::read_analog_stream(applied.frequency_hz, &mut read_buffers)?;
            let count = chunk.channels[0].len();
            if count > 0 || chunk.samples_lost > 0 || chunk.samples_corrupt > 0 {
                last_data = Instant::now();
                packed.clear();
                packed.reserve(count * 16);
                for i in 0..count {
                    packed.extend_from_slice(&chunk.channels[0][i].to_le_bytes());
                    packed.extend_from_slice(&chunk.channels[1][i].to_le_bytes());
                }
                data_log.write_all(&packed).map_err(err)?;
                let mut s = state.lock().map_err(err)?;
                let sample_start = s.total.saturating_add(chunk.samples_lost as u64);
                let display_bucket_size = s.display_bucket_size;
                let display_capacity = s.display_capacity;
                s.total = sample_start;
                writeln!(chunks, "{}", serde_json::json!({"sample_start":sample_start,"count":count,"lost_before":chunk.samples_lost,"corrupt":chunk.samples_corrupt,"file_sample_offset":s.received})).map_err(err)?;
                s.total += count as u64;
                s.received += count as u64;
                s.lost += chunk.samples_lost as u64;
                s.corrupt += chunk.samples_corrupt as u64;
                let capacity = s.capacity;
                append_window(
                    &mut s.channels,
                    &chunk.channels,
                    capacity,
                    chunk.samples_lost > 0,
                    chunk.samples_corrupt > 0,
                );
                append_display(
                    &mut s.display,
                    &chunk.channels,
                    sample_start,
                    display_bucket_size,
                    display_capacity,
                    chunk.samples_lost > 0,
                    chunk.samples_corrupt > 0,
                );
                s.sequence += 1;
                let mut channels = chunk.channels.into_iter();
                read_buffers = [
                    channels.next().unwrap_or_default(),
                    channels.next().unwrap_or_default(),
                ];
            } else {
                if last_data.elapsed() > Duration::from_secs(5) {
                    return Err("No samples received from AD3 for 5 seconds".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            if last_flush.elapsed() > Duration::from_secs(1) {
                data_log.flush().map_err(err)?;
                chunks.flush().map_err(err)?;
                last_flush = Instant::now();
            }
        }
        data_log.flush().map_err(err)?;
        chunks.flush().map_err(err)?;
        let s = state.lock().map_err(err)?;
        fs::write(directory.join("summary.json"), serde_json::to_vec_pretty(&serde_json::json!({"received_samples":s.received,"timeline_samples":s.total,"lost":s.lost,"corrupt":s.corrupt,"stopped":true})).map_err(err)?).map_err(err)?;
        // A small CSV of the final visible window is convenient for inspection.
        let mut csv = BufWriter::new(File::create(directory.join("last-window.csv")).map_err(err)?);
        writeln!(csv, "time_s,A1_V,A2_V").map_err(err)?;
        let origin = s.total.saturating_sub(s.channels[0].len() as u64);
        for (i, (a, b)) in s.channels[0].iter().zip(&s.channels[1]).enumerate() {
            writeln!(
                csv,
                "{:.12},{:.9},{:.9}",
                (origin + i as u64) as f64 / s.rate,
                a,
                b
            )
            .map_err(err)?;
        }
        csv.flush().map_err(err)?;
        Ok(())
    })();
    let halted = dwf::set_analog_stream_running(false);
    let closed = dwf::close_ad3();
    result.and(halted).and(closed)
}
fn append_window(
    window: &mut [VecDeque<f64>; 2],
    channels: &[Vec<f64>],
    capacity: usize,
    lost: bool,
    corrupt: bool,
) {
    if lost || corrupt {
        for channel in window.iter_mut() {
            channel.clear();
        }
    }
    if corrupt {
        return;
    }
    for (target, source) in window.iter_mut().zip(channels) {
        let offset = source.len().saturating_sub(capacity);
        let overflow = (target.len() + source.len() - offset).saturating_sub(capacity);
        target.drain(..overflow.min(target.len()));
        target.extend(&source[offset..]);
    }
}

fn append_display(
    display: &mut VecDeque<DisplayBucket>,
    channels: &[Vec<f64>],
    sample_start: u64,
    bucket_size: usize,
    capacity: usize,
    lost: bool,
    corrupt: bool,
) {
    if lost || corrupt {
        display.clear();
    }
    if corrupt || channels.len() < 2 {
        return;
    }
    let count = channels[0].len().min(channels[1].len());
    for index in 0..count {
        let sample_index = sample_start.saturating_add(index as u64);
        let values = [channels[0][index], channels[1][index]];
        let append_to_last = display.back().is_some_and(|bucket| {
            bucket.sample_start + bucket.count as u64 == sample_index && bucket.count < bucket_size
        });
        if append_to_last {
            let bucket = display.back_mut().expect("display bucket exists");
            for channel in 0..2 {
                bucket.min[channel] = bucket.min[channel].min(values[channel]);
                bucket.max[channel] = bucket.max[channel].max(values[channel]);
            }
            bucket.count += 1;
        } else {
            display.push_back(DisplayBucket {
                sample_start: sample_index,
                count: 1,
                min: values,
                max: values,
            });
        }
        while display.len() > capacity {
            display.pop_front();
        }
    }
}
#[tauri::command]
pub async fn stop_stream() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(stop_blocking)
        .await
        .map_err(err)?
}
pub fn stop_blocking() -> Result<(), String> {
    let mut manager = control().lock().map_err(err)?;
    if let Some(control) = manager.as_mut() {
        control.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = control.worker.take() {
            worker.join().map_err(|_| "Stream worker panicked")?;
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn stream_snapshot(
    start_s: Option<f64>,
    span_s: Option<f64>,
    pixels: usize,
) -> Result<StreamView, String> {
    tauri::async_runtime::spawn_blocking(move || snapshot(start_s, span_s, pixels))
        .await
        .map_err(err)?
}
fn snapshot(
    start_s: Option<f64>,
    span_s: Option<f64>,
    pixels: usize,
) -> Result<StreamView, String> {
    let shared = shared()?;
    let s = shared.lock().map_err(err)?;
    let display: Vec<DisplayBucket> = s.display.iter().cloned().collect();
    let count = display.iter().map(|bucket| bucket.count).sum::<usize>();
    let origin_sample = display
        .first()
        .map(|bucket| bucket.sample_start)
        .unwrap_or(s.total);
    let origin = origin_sample as f64 / s.rate;
    let display_end = display
        .last()
        .map(|bucket| bucket.sample_start.saturating_add(bucket.count as u64))
        .unwrap_or(origin_sample);
    let duration = display_end.saturating_sub(origin_sample) as f64 / s.rate;
    let stats = (0..2)
        .map(|channel| {
            let min_v = display
                .iter()
                .map(|bucket| bucket.min[channel])
                .reduce(f64::min)
                .unwrap_or(0.0);
            let max_v = display
                .iter()
                .map(|bucket| bucket.max[channel])
                .reduce(f64::max)
                .unwrap_or(0.0);
            SignalStats {
                min_v,
                max_v,
                mean_v: (min_v + max_v) / 2.0,
            }
        })
        .collect();
    let mut view = StreamView {
        running: s.running,
        directory: s.directory.clone(),
        sample_rate_hz: s.rate,
        adc_bits: s.bits,
        total_samples: s.total,
        samples_lost: s.lost,
        samples_corrupt: s.corrupt,
        sample_count: count,
        origin_s: origin,
        duration_s: duration,
        sequence: s.sequence,
        error: s.error.clone(),
        envelopes: [
            Envelope {
                start_s: 0.0,
                step_s: 0.0,
                min: vec![],
                max: vec![],
            },
            Envelope {
                start_s: 0.0,
                step_s: 0.0,
                min: vec![],
                max: vec![],
            },
        ],
        stats,
    };
    drop(s);
    let start = start_s.unwrap_or(0.0).max(0.0);
    let span = span_s
        .unwrap_or(view.duration_s)
        .max(1.0 / view.sample_rate_hz);
    for i in 0..2 {
        view.envelopes[i] = envelope_display(
            &display,
            i,
            view.sample_rate_hz,
            origin,
            start,
            span,
            pixels.clamp(64, 4096),
        );
    }
    Ok(view)
}

fn envelope_display(
    display: &[DisplayBucket],
    channel: usize,
    rate: f64,
    origin: f64,
    start: f64,
    span: f64,
    pixels: usize,
) -> Envelope {
    let range_start = ((origin + start).max(0.0) * rate).floor() as u64;
    let range_end = ((origin + start + span).max(0.0) * rate).ceil() as u64;
    let first = display
        .iter()
        .position(|bucket| bucket.sample_start.saturating_add(bucket.count as u64) > range_start);
    let Some(first) = first else {
        return Envelope {
            start_s: 0.0,
            step_s: 0.0,
            min: vec![],
            max: vec![],
        };
    };
    let last = display[first..]
        .iter()
        .position(|bucket| bucket.sample_start >= range_end)
        .map(|offset| first + offset)
        .unwrap_or(display.len());
    if first >= last {
        return Envelope {
            start_s: 0.0,
            step_s: 0.0,
            min: vec![],
            max: vec![],
        };
    }
    let step_buckets = (last - first).div_ceil(pixels).max(1);
    let mut result = Envelope {
        start_s: display[first].sample_start as f64 / rate - origin,
        step_s: 0.0,
        min: vec![],
        max: vec![],
    };
    for group in display[first..last].chunks(step_buckets) {
        result.min.push(
            group
                .iter()
                .map(|bucket| bucket.min[channel])
                .fold(f64::INFINITY, f64::min),
        );
        result.max.push(
            group
                .iter()
                .map(|bucket| bucket.max[channel])
                .fold(f64::NEG_INFINITY, f64::max),
        );
    }
    result.step_s = step_buckets as f64 * group_bucket_size(display, first) as f64 / rate;
    result
}

fn group_bucket_size(display: &[DisplayBucket], first: usize) -> usize {
    display
        .get(first)
        .and_then(|bucket| {
            display
                .get(first + 1)
                .map(|next| next.sample_start - bucket.sample_start)
        })
        .unwrap_or(display[first].count as u64)
        .max(1) as usize
}
fn envelope(values: &[f64], rate: f64, start: f64, span: f64, pixels: usize) -> Envelope {
    let first = ((start * rate).floor() as usize).min(values.len());
    let last = (((start + span) * rate).ceil() as usize).min(values.len());
    let step = last.saturating_sub(first).div_ceil(pixels).max(1);
    let mut result = Envelope {
        start_s: first as f64 / rate,
        step_s: step as f64 / rate,
        min: vec![],
        max: vec![],
    };
    for chunk in values[first..last.max(first)].chunks(step) {
        result
            .min
            .push(chunk.iter().copied().fold(f64::INFINITY, f64::min));
        result
            .max
            .push(chunk.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    }
    result
}
#[tauri::command]
pub async fn decode_stream(configs: Vec<Option<DecodeConfig>>) -> Result<StreamDecoded, String> {
    if configs.len() != 2 {
        return Err("Two decoder configurations are required".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let shared = shared()?;
        let s = shared.lock().map_err(err)?;
        if s.channels[0].is_empty() {
            return Err("Waiting for clean analog samples".into());
        }
        let origin = s.total.saturating_sub(s.channels[0].len() as u64) as f64 / s.rate;
        let sequence = s.sequence;
        let logged = LoggedCapture {
            capture: dwf::AnalogCapture {
                sample_rate_hz: s.rate,
                channels: s
                    .channels
                    .iter()
                    .map(|c| c.iter().copied().collect())
                    .collect(),
                samples_lost: 0,
                samples_corrupt: 0,
            },
            adc_bits: s.bits,
            directory: s.directory.clone(),
            stats: vec![],
        };
        drop(s);
        let results = configs
            .into_iter()
            .map(|config| match config {
                Some(config) => analyzer::decode_logged(&logged, &config),
                None => Err("Decoder off".into()),
            })
            .collect();
        Ok(StreamDecoded {
            origin_s: origin,
            sequence,
            results,
        })
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a connected AD3"]
    fn live_stream_start_stop_and_restart() {
        let device = dwf::discover_ad3()
            .unwrap()
            .into_iter()
            .next()
            .expect("AD3 attached");
        for pass in 0..2 {
            start(device.index, 1_000_000.0, 100.0).unwrap();
            thread::sleep(Duration::from_millis(500));
            let first = snapshot(None, None, 1000).unwrap();
            thread::sleep(Duration::from_millis(500));
            let second = snapshot(None, None, 1000).unwrap();
            let stop_at = Instant::now();
            stop_blocking().unwrap();
            let stopped = snapshot(None, None, 1000).unwrap();
            println!(
                "pass={pass} first={} second={} stopped={} lost={} corrupt={} stop_ms={} log={}",
                first.total_samples,
                second.total_samples,
                stopped.total_samples,
                stopped.samples_lost,
                stopped.samples_corrupt,
                stop_at.elapsed().as_millis(),
                stopped.directory
            );
            assert!(first.running && second.running);
            assert!(second.total_samples > first.total_samples);
            assert!(!stopped.running);
            assert!(stopped.error.is_none(), "{:?}", stopped.error);
            assert!(stopped.sample_count <= 100_000);
            assert!(stopped
                .envelopes
                .iter()
                .all(|e| !e.min.is_empty() && e.min.len() <= 1000));
            assert!(PathBuf::from(&stopped.directory)
                .join("last-window.csv")
                .exists());
            thread::sleep(Duration::from_millis(50));
            assert_eq!(
                snapshot(None, None, 1000).unwrap().total_samples,
                stopped.total_samples
            );
        }
    }
    #[test]
    fn bounded_window_keeps_latest_and_discards_gaps() {
        let mut window = [VecDeque::new(), VecDeque::new()];
        append_window(
            &mut window,
            &[vec![1., 2., 3.], vec![11., 12., 13.]],
            4,
            false,
            false,
        );
        append_window(
            &mut window,
            &[vec![4., 5., 6.], vec![14., 15., 16.]],
            4,
            false,
            false,
        );
        assert_eq!(
            window[0].iter().copied().collect::<Vec<_>>(),
            vec![3., 4., 5., 6.]
        );
        append_window(&mut window, &[vec![9.], vec![19.]], 4, true, false);
        assert_eq!(window[1].iter().copied().collect::<Vec<_>>(), vec![19.]);
        append_window(&mut window, &[vec![8.], vec![18.]], 4, false, true);
        assert!(window[0].is_empty());
    }
    #[test]
    fn high_rate_live_window_uses_display_buckets() {
        let samples = window_sample_count(100_000_000.0, 100.0).unwrap();
        assert_eq!(samples, 10_000_000);
        let (bucket_size, capacity) = display_config(samples);
        assert!(bucket_size > 1);
        assert!(capacity <= MAX_DISPLAY_BUCKETS);

        let mut display = VecDeque::new();
        append_display(
            &mut display,
            &[vec![0.0, 3.3, 0.0], vec![1.0, 1.0, 1.0]],
            100,
            bucket_size,
            capacity,
            false,
            false,
        );
        let display_view: Vec<_> = display.iter().cloned().collect();
        let result = envelope_display(
            &display_view,
            0,
            100_000_000.0,
            100.0 / 100_000_000.0,
            0.0,
            3.0 / 100_000_000.0,
            100,
        );
        assert_eq!(result.max.iter().copied().fold(0.0, f64::max), 3.3);
    }
    #[test]
    fn envelopes_preserve_single_sample_pulses_and_zoom() {
        let mut values = vec![0.; 10000];
        values[5001] = 3.3;
        let result = envelope(&values, 1e6, 0., 0.01, 500);
        assert!(result.min.len() <= 500);
        assert_eq!(result.max.iter().copied().fold(0., f64::max), 3.3);
        let zoom = envelope(&values, 1e6, 0.005, 0.00001, 500);
        assert_eq!(zoom.step_s, 1e-6);
        assert_eq!(zoom.max[1], 3.3);
    }
    #[test]
    fn stop_without_start_is_safe() {
        stop_blocking().unwrap();
    }
}
