use crate::dwf::AnalogCapture;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize)]
pub struct LoggedCapture {
    pub capture: AnalogCapture,
    pub adc_bits: i32,
    pub directory: String,
    pub stats: Vec<SignalStats>,
}
#[derive(Clone, Serialize)]
pub struct SignalStats {
    pub min_v: f64,
    pub max_v: f64,
    pub mean_v: f64,
}
pub(crate) static ACQUISITION: Mutex<()> = Mutex::new(());
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn unique_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}
fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}
fn decoder_executable() -> PathBuf {
    std::env::var_os("SIGROK_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join(".tools/sigrok-cli/sigrok-cli.exe"))
}
fn command() -> Command {
    let mut cmd = Command::new(decoder_executable());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}

#[derive(Serialize)]
pub struct DecoderInfo {
    id: String,
    name: String,
}
#[tauri::command]
pub async fn available_decoders() -> Result<Vec<DecoderInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let output = command().arg("--list-supported").output().map_err(|e| {
            format!(
                "Decoder backend unavailable: {e}. Set SIGROK_CLI to your sigrok-cli executable."
            )
        })?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let section = text
            .split("Supported protocol decoders:")
            .nth(1)
            .ok_or("No decoder catalog returned")?;
        Ok(section
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let (id, name) = line.split_once(char::is_whitespace)?;
                Some(DecoderInfo {
                    id: id.into(),
                    name: name.trim().into(),
                })
            })
            .collect())
    })
    .await
    .map_err(error)?
}
#[tauri::command]
pub async fn decoder_details(id: String) -> Result<String, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Invalid decoder ID".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let output = command()
            .args(["--show", "-P", &id])
            .output()
            .map_err(error)?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    })
    .await
    .map_err(error)?
}

#[derive(Deserialize, Serialize, Clone)]
pub struct DecodeConfig {
    pub expression: String,
    pub thresholds: [f64; 2],
    pub hysteresis_v: f64,
    pub differential: bool,
}
#[derive(Serialize)]
pub struct DecodeResult {
    pub annotations: Vec<Annotation>,
    pub output: String,
    pub diagnostics: String,
    pub directory: String,
    pub transitions: [usize; 2],
    pub truncated: bool,
}

fn digitize(
    capture: &AnalogCapture,
    config: &DecodeConfig,
) -> Result<(Vec<u8>, [usize; 2]), String> {
    if capture.samples_lost != 0 || capture.samples_corrupt != 0 {
        return Err(
            "Capture has lost/corrupt samples; recapture at a lower sample rate before decoding."
                .into(),
        );
    }
    if config.thresholds.iter().any(|v| !v.is_finite())
        || !config.hysteresis_v.is_finite()
        || config.hysteresis_v < 0.0
    {
        return Err("Thresholds must be finite and hysteresis must be non-negative".into());
    }
    if capture.channels.len() != 2
        || capture.channels[0].is_empty()
        || capture.channels[0].len() != capture.channels[1].len()
    {
        return Err("Two equal-length analog channels are required".into());
    }
    let mut bytes = Vec::with_capacity(capture.channels[0].len());
    let mut states = [false; 2];
    let mut transitions = [0; 2];
    for i in 0..capture.channels[0].len() {
        let a1 = capture.channels[0][i];
        let a2 = capture.channels[1][i];
        for (ch, value) in [if config.differential { a1 - a2 } else { a1 }, a2]
            .iter()
            .enumerate()
        {
            if !value.is_finite() {
                return Err("Capture contains non-finite samples".into());
            }
            let previous = states[ch];
            if i == 0 {
                states[ch] = *value >= config.thresholds[ch];
            } else if *value >= config.thresholds[ch] + config.hysteresis_v / 2.0 {
                states[ch] = true;
            } else if *value <= config.thresholds[ch] - config.hysteresis_v / 2.0 {
                states[ch] = false;
            }
            if i > 0 && states[ch] != previous {
                transitions[ch] += 1;
            }
        }
        // Differential CAN input is dominant-high; CAN RX decoders expect dominant-low.
        bytes.push(u8::from(states[0] ^ config.differential) | (u8::from(states[1]) << 1));
    }
    Ok((bytes, transitions))
}

pub(crate) fn decode_logged(
    capture: &LoggedCapture,
    config: &DecodeConfig,
) -> Result<DecodeResult, String> {
    if config.expression.is_empty()
        || config.expression.len() > 4096
        || config.expression.chars().any(char::is_control)
    {
        return Err("Enter a valid decoder expression".into());
    }
    let directory = &capture.directory;
    let (bytes, transitions) = digitize(&capture.capture, config)?;
    let path = PathBuf::from(&directory).join(format!("decode-{}", unique_id()));
    fs::create_dir(&path).map_err(error)?;
    let input = path.join("logic.bin");
    fs::write(&input, bytes).map_err(error)?;
    fs::write(
        path.join("settings.json"),
        serde_json::to_vec_pretty(&config).map_err(error)?,
    )
    .map_err(error)?;
    let mut child = command()
        .args(["-i"])
        .arg(&input)
        .args([
            "-I",
            &format!(
                "binary:numchannels=2:samplerate={:.0}",
                capture.capture.sample_rate_hz
            ),
            "-P",
            &config.expression,
            "--protocol-decoder-jsontrace",
        ])
        .stdout(Stdio::from(
            File::create(path.join("annotations.json")).map_err(error)?,
        ))
        .stderr(Stdio::from(
            File::create(path.join("diagnostics.txt")).map_err(error)?,
        ))
        .spawn()
        .map_err(error)?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(error)? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Decoder timed out after 30 seconds; output retained with capture".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let diagnostics = fs::read_to_string(path.join("diagnostics.txt")).map_err(error)?;
    if !status.success() {
        return Err(format!("Decoder failed: {diagnostics}"));
    }
    let full = fs::read_to_string(path.join("annotations.json")).map_err(error)?;
    let annotations = parse_annotations(&full)?;
    Ok(DecodeResult {
        annotations,
        truncated: full.len() > 64000,
        output: full.chars().take(64000).collect(),
        diagnostics,
        directory: path.to_string_lossy().into_owned(),
        transitions,
    })
}

#[derive(Serialize, Debug)]
pub struct Annotation {
    pub start_s: f64,
    pub end_s: f64,
    pub row: String,
    pub text: String,
    pub value: Option<u32>,
}
fn parse_annotations(text: &str) -> Result<Vec<Annotation>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("Invalid decoder annotations: {e}"))?;
    let events = json["traceEvents"]
        .as_array()
        .ok_or("Decoder returned no traceEvents array")?;
    let mut starts: std::collections::HashMap<String, Vec<(f64, String)>> =
        std::collections::HashMap::new();
    let mut annotations = Vec::new();
    for event in events {
        let row = event["tid"].as_str().unwrap_or("");
        let pid = event["pid"].as_str().unwrap_or("");
        let key = format!("{pid}/{row}");
        let ts = event["ts"].as_f64().ok_or("Missing annotation time")? / 1e6;
        let name = event["name"].as_str().unwrap_or("").to_owned();
        if event["ph"] == "B" {
            starts.entry(key).or_default().push((ts, name));
        } else if event["ph"] == "E" {
            if let Some((start_s, text)) = starts.get_mut(&key).and_then(Vec::pop) {
                // Bit rows are redundant with the signal; keep data, fields and warnings.
                if row.to_lowercase().contains("bits") {
                    continue;
                }
                let hex = text.strip_prefix("0x").unwrap_or(&text);
                let data_row = row == "RX"
                    || row == "TX"
                    || row.to_lowercase().contains("data")
                    || row == "MOSI"
                    || row == "MISO";
                let value = if data_row
                    && !hex.is_empty()
                    && hex.len() <= 2
                    && hex.chars().all(|c| c.is_ascii_hexdigit())
                {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    None
                };
                annotations.push(Annotation {
                    start_s,
                    end_s: ts,
                    row: row.into(),
                    text,
                    value,
                });
            }
        }
    }
    annotations.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    Ok(annotations)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_hysteresis_and_channel_order() {
        let capture = AnalogCapture {
            sample_rate_hz: 1e6,
            channels: vec![vec![0.0, 2.0, 0.95, 0.0], vec![2.0, 0.0, 0.0, 2.0]],
            samples_lost: 0,
            samples_corrupt: 0,
        };
        let config = DecodeConfig {
            expression: "uart:rx=0:tx=1".into(),
            thresholds: [1.0, 1.0],
            hysteresis_v: 0.2,
            differential: false,
        };
        let (bytes, transitions) = digitize(&capture, &config).unwrap();
        assert_eq!(bytes, vec![2, 1, 1, 2]);
        assert_eq!(transitions, [2, 2]);
        let mut damaged = capture.clone();
        damaged.samples_lost = 1;
        assert!(digitize(&damaged, &config).is_err());
    }
    #[test]
    fn differential_can_is_dominant_low() {
        let capture = AnalogCapture {
            sample_rate_hz: 1e6,
            channels: vec![vec![2.5, 3.5, 2.5], vec![2.5, 1.5, 2.5]],
            samples_lost: 0,
            samples_corrupt: 0,
        };
        let config = DecodeConfig {
            expression: "can:can_rx=0".into(),
            thresholds: [0.9, 1.0],
            hysteresis_v: 0.1,
            differential: true,
        };
        let (bytes, _) = digitize(&capture, &config).unwrap();
        assert_eq!(
            bytes.iter().map(|b| b & 1).collect::<Vec<_>>(),
            vec![1, 0, 1]
        );
    }
}
