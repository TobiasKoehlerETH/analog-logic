use crate::dwf::AnalogCapture;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const IMU_FIRMWARE_DECODER: &str = "imu_firmware";
const IMU_FIRMWARE_EXPRESSION_PREFIX: &str = "imu_firmware:";

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
    if id == IMU_FIRMWARE_DECODER {
        return Ok("IMU firmware application protocol\n\n\
             Underlying decoder: Classical CAN\n\
             Nominal bitrate: configured bit rate (default 1,000,000 bit/s)\n\
             0x100: ax, ay, az as signed big-endian i16 in mg\n\
             0x101: gx, gy as signed big-endian i32 in mdps\n\
             0x102: gz as signed big-endian i32 in mdps\n\
             One decoded sample is emitted after all three frames arrive."
            .into());
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

fn sigrok_expression(expression: &str) -> Result<String, String> {
    let Some(settings) = expression.strip_prefix(IMU_FIRMWARE_EXPRESSION_PREFIX) else {
        return Ok(expression.into());
    };
    let mut channel = None;
    let mut bitrate = None;
    for setting in settings.split(':') {
        if let Some(value) = setting.strip_prefix("can_rx=") {
            channel = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| "IMU firmware channel must be 0 or 1")?,
            );
        } else if let Some(value) = setting.strip_prefix("nominal_bitrate=") {
            bitrate = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| "IMU firmware bitrate must be a positive integer")?,
            );
        }
    }
    let channel = channel.ok_or("IMU firmware decoder requires can_rx=<channel>")?;
    if channel > 1 {
        return Err("IMU firmware channel must be 0 or 1".into());
    }
    let bitrate = bitrate.ok_or("IMU firmware decoder requires nominal_bitrate=<bit/s>")?;
    if bitrate == 0 {
        return Err("IMU firmware bitrate must be a positive integer".into());
    }
    Ok(format!("can:can_rx={channel}:nominal_bitrate={bitrate}"))
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
        for (ch, value) in [if config.differential { a2 - a1 } else { a1 }, a2]
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
        // CAN-L on A1 and CAN-H on A2 form a dominant-high differential signal;
        // CAN RX decoders expect dominant-low.
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
    let decoder_expression = sigrok_expression(&config.expression)?;
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
            &decoder_expression,
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
    let annotations = if config
        .expression
        .starts_with(IMU_FIRMWARE_EXPRESSION_PREFIX)
    {
        decode_imu_firmware(&annotations)
    } else {
        annotations
    };
    Ok(DecodeResult {
        annotations,
        truncated: full.len() > 64000,
        output: full.chars().take(64000).collect(),
        diagnostics,
        directory: path.to_string_lossy().into_owned(),
        transitions,
    })
}

#[derive(Clone, Serialize, Debug)]
pub struct Annotation {
    pub start_s: f64,
    pub end_s: f64,
    pub row: String,
    pub text: String,
    pub value: Option<u32>,
}

#[derive(Clone)]
struct CanFrame {
    id: u32,
    length: Option<usize>,
    remote: bool,
    start_s: f64,
    end_s: f64,
    data: [Option<u8>; 8],
}

fn parse_identifier(text: &str) -> Option<u32> {
    let value = text.strip_prefix("Identifier:")?.trim();
    let (digits, radix) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or((value, 10), |digits| (digits, 16));
    u32::from_str_radix(digits, radix).ok()
}

fn parse_data_byte(text: &str) -> Option<(usize, u8)> {
    let (label, value) = text.split_once(':')?;
    let index = label.strip_prefix("Data byte ")?.parse::<usize>().ok()?;
    let value = value.trim();
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    Some((index, u8::from_str_radix(digits, 16).ok()?))
}

fn collect_can_frames(annotations: &[Annotation]) -> Vec<CanFrame> {
    let mut frames = Vec::new();
    let mut current: Option<CanFrame> = None;
    for annotation in annotations {
        if let Some(id) = parse_identifier(&annotation.text) {
            if let Some(frame) = current.take() {
                frames.push(frame);
            }
            current = Some(CanFrame {
                id,
                length: None,
                remote: false,
                start_s: annotation.start_s,
                end_s: annotation.end_s,
                data: [None; 8],
            });
            continue;
        }
        if let Some(length) = annotation.text.strip_prefix("Data length code:") {
            if let Ok(length) = length.trim().parse::<usize>() {
                if let Some(frame) = current.as_mut() {
                    frame.length = Some(length);
                }
            }
        }
        if annotation
            .text
            .to_ascii_lowercase()
            .contains("remote frame")
        {
            if let Some(frame) = current.as_mut() {
                frame.remote = true;
            }
        }
        if let Some((index, value)) = parse_data_byte(&annotation.text) {
            if let Some(frame) = current.as_mut() {
                if index < frame.data.len() {
                    frame.data[index] = Some(value);
                }
            }
        }
        if let Some(frame) = current.as_mut() {
            frame.end_s = annotation.end_s;
        }
    }
    if let Some(frame) = current {
        frames.push(frame);
    }
    frames
}

fn frame_payload(frame: &CanFrame, length: usize) -> Option<Vec<u8>> {
    if frame.remote || frame.length != Some(length) || length > frame.data.len() {
        return None;
    }
    frame.data[..length]
        .iter()
        .copied()
        .collect::<Option<Vec<u8>>>()
}

fn warning(start_s: f64, end_s: f64, text: String) -> Annotation {
    Annotation {
        start_s,
        end_s,
        row: "Warnings".into(),
        text,
        value: None,
    }
}

fn semantic(start_s: f64, end_s: f64, text: String) -> Annotation {
    Annotation {
        start_s,
        end_s,
        row: "IMU firmware".into(),
        text,
        value: None,
    }
}

fn decode_imu_firmware(annotations: &[Annotation]) -> Vec<Annotation> {
    let mut decoded: Vec<Annotation> = annotations
        .iter()
        .filter(|annotation| {
            annotation.row.to_ascii_lowercase().contains("warning")
                || annotation.row.to_ascii_lowercase().contains("error")
        })
        .cloned()
        .collect();
    let mut pending: [Option<CanFrame>; 3] = [None, None, None];
    let mut sample_index = 0;

    for frame in collect_can_frames(annotations) {
        let (slot, expected_length, name) = match frame.id {
            0x100 => (0, 6, "acceleration"),
            0x101 => (1, 8, "gyro X/Y"),
            0x102 => (2, 4, "gyro Z"),
            _ => continue,
        };
        if frame.remote {
            continue;
        }
        if frame_payload(&frame, expected_length).is_none() {
            decoded.push(warning(
                frame.start_s,
                frame.end_s,
                format!(
                    "Invalid IMU firmware frame 0x{:03X}: {name} requires {expected_length} data bytes",
                    frame.id
                ),
            ));
            if slot == 0 {
                pending = [None, None, None];
            }
            continue;
        }

        if slot == 0 {
            pending = [Some(frame), None, None];
            continue;
        }
        if pending[0].is_none() {
            continue;
        }
        pending[slot] = Some(frame);
        if pending.iter().any(Option::is_none) {
            continue;
        }

        let (Some(accel), Some(gyro_xy), Some(gyro_z)) =
            (pending[0].take(), pending[1].take(), pending[2].take())
        else {
            continue;
        };
        let accel_start = accel.start_s;
        let accel_end = accel.end_s;
        let gyro_xy_start = gyro_xy.start_s;
        let gyro_xy_end = gyro_xy.end_s;
        let gyro_z_start = gyro_z.start_s;
        let gyro_z_end = gyro_z.end_s;
        let Some(accel_data) = frame_payload(&accel, 6) else {
            continue;
        };
        let Some(gyro_xy_data) = frame_payload(&gyro_xy, 8) else {
            continue;
        };
        let Some(gyro_z_data) = frame_payload(&gyro_z, 4) else {
            continue;
        };

        let ax = i16::from_be_bytes([accel_data[0], accel_data[1]]);
        let ay = i16::from_be_bytes([accel_data[2], accel_data[3]]);
        let az = i16::from_be_bytes([accel_data[4], accel_data[5]]);
        let gx = i32::from_be_bytes([
            gyro_xy_data[0],
            gyro_xy_data[1],
            gyro_xy_data[2],
            gyro_xy_data[3],
        ]);
        let gy = i32::from_be_bytes([
            gyro_xy_data[4],
            gyro_xy_data[5],
            gyro_xy_data[6],
            gyro_xy_data[7],
        ]);
        let gz = i32::from_be_bytes([
            gyro_z_data[0],
            gyro_z_data[1],
            gyro_z_data[2],
            gyro_z_data[3],
        ]);
        sample_index += 1;
        decoded.push(semantic(
            accel_start,
            accel_end,
            format!("IMU #{sample_index} · ax={ax} mg · ay={ay} mg · az={az} mg"),
        ));
        decoded.push(semantic(
            gyro_xy_start,
            gyro_xy_end,
            format!("IMU #{sample_index} · gx={gx} mdps · gy={gy} mdps"),
        ));
        decoded.push(semantic(
            gyro_z_start,
            gyro_z_end,
            format!("IMU #{sample_index} · gz={gz} mdps"),
        ));
    }
    decoded.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    decoded
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
            channels: vec![vec![2.5, 1.5, 2.5], vec![2.5, 3.5, 2.5]],
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

    #[test]
    fn imu_firmware_expression_uses_can_decoder_at_configured_bitrate() {
        assert_eq!(
            sigrok_expression("imu_firmware:can_rx=1:nominal_bitrate=1000000").unwrap(),
            "can:can_rx=1:nominal_bitrate=1000000"
        );
        assert!(sigrok_expression("imu_firmware:can_rx=0:nominal_bitrate=0").is_err());
    }

    #[test]
    fn imu_firmware_reassembles_signed_big_endian_payloads() {
        let texts = [
            "Identifier: 256",
            "Data length code: 6",
            "Data byte 0: 0x00",
            "Data byte 1: 0x13",
            "Data byte 2: 0xFF",
            "Data byte 3: 0xE1",
            "Data byte 4: 0x03",
            "Data byte 5: 0xFC",
            "Identifier: 0x101",
            "Data length code: 8",
            "Data byte 0: 0xFF",
            "Data byte 1: 0xFF",
            "Data byte 2: 0xFA",
            "Data byte 3: 0x24",
            "Data byte 4: 0x00",
            "Data byte 5: 0x00",
            "Data byte 6: 0x09",
            "Data byte 7: 0xC4",
            "Identifier: 258",
            "Data length code: 4",
            "Data byte 0: 0xFF",
            "Data byte 1: 0xFF",
            "Data byte 2: 0xF4",
            "Data byte 3: 0x48",
        ];
        let annotations: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| Annotation {
                start_s: index as f64 * 0.00001,
                end_s: (index + 1) as f64 * 0.00001,
                row: "Fields".into(),
                text: (*text).into(),
                value: None,
            })
            .collect();
        let decoded = decode_imu_firmware(&annotations);
        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded[0].text,
            "IMU #1 · ax=19 mg · ay=-31 mg · az=1020 mg"
        );
        assert_eq!(decoded[1].text, "IMU #1 · gx=-1500 mdps · gy=2500 mdps");
        assert_eq!(decoded[2].text, "IMU #1 · gz=-3000 mdps");
    }

    #[test]
    fn imu_firmware_ignores_remote_requests() {
        let texts = [
            "Identifier: 256",
            "Data length code: 6",
            "Remote transmission request: remote frame",
        ];
        let annotations: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| Annotation {
                start_s: index as f64 * 0.00001,
                end_s: (index + 1) as f64 * 0.00001,
                row: "Fields".into(),
                text: (*text).into(),
                value: None,
            })
            .collect();
        assert!(decode_imu_firmware(&annotations).is_empty());
    }
}
