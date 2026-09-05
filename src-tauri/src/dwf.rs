// WaveForms ABI and acquisition lifecycle adapted from dwfpy (MIT).
// See THIRD_PARTY_NOTICES.md for attribution.
use libloading::{Library, Symbol};
use serde::{Deserialize, Serialize};
use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::{Mutex, OnceLock};
static SESSION: OnceLock<Mutex<Option<DeviceSession>>> = OnceLock::new();

const DEVICE_ID_DISCOVERY_3: i32 = 10;
const DWF_STATE_DONE: i8 = 2;
const FILTER_DECIMATE: i32 = 0;
const TRIGSRC_NONE: i32 = 0;
const TRIGSRC_DETECTOR_ANALOG_IN: i32 = 2;
const TRIGTYPE_EDGE: i32 = 0;
type VersionFn = unsafe extern "C" fn(*mut c_char) -> i32;
type EnumFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type EnumTypeFn = unsafe extern "C" fn(i32, *mut i32, *mut i32) -> i32;
type EnumUsedFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type EnumTextFn = unsafe extern "C" fn(i32, *mut c_char) -> i32;
type EnumConfigFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type ErrorTextFn = unsafe extern "C" fn(*mut c_char) -> i32;
type DeviceOpenFn = unsafe extern "C" fn(i32, i32, *mut i32) -> i32;
type DeviceOpenDefaultFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type DeviceCloseFn = unsafe extern "C" fn(i32) -> i32;
type DeviceResetFn = unsafe extern "C" fn(i32) -> i32;
type DeviceAutoConfigFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogChannelCountFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type AnalogBitsInfoFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type AnalogFrequencyInfoFn = unsafe extern "C" fn(i32, *mut f64, *mut f64) -> i32;
type AnalogBufferInfoFn = unsafe extern "C" fn(i32, *mut i32, *mut i32) -> i32;
type AnalogModeInfoFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type AnalogSetChannelFn = unsafe extern "C" fn(i32, i32, i32) -> i32;
type AnalogSetRangeFn = unsafe extern "C" fn(i32, i32, f64) -> i32;
type AnalogSetOffsetFn = unsafe extern "C" fn(i32, i32, f64) -> i32;
type AnalogSetAttenuationFn = unsafe extern "C" fn(i32, i32, f64) -> i32;
type AnalogSetBandwidthFn = unsafe extern "C" fn(i32, i32, f64) -> i32;
type AnalogSetImpedanceFn = unsafe extern "C" fn(i32, i32, f64) -> i32;
type AnalogSetCouplingFn = unsafe extern "C" fn(i32, i32, i32) -> i32;
type AnalogSetFrequencyFn = unsafe extern "C" fn(i32, f64) -> i32;
type AnalogSetBufferFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogSetModeFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogSetFilterFn = unsafe extern "C" fn(i32, i32, i32) -> i32;
type AnalogRecordLengthFn = unsafe extern "C" fn(i32, f64) -> i32;
type AnalogFrequencyGetFn = unsafe extern "C" fn(i32, *mut f64) -> i32;
type AnalogBufferGetFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type AnalogModeGetFn = unsafe extern "C" fn(i32, *mut i32) -> i32;
type AnalogRecordLengthGetFn = unsafe extern "C" fn(i32, *mut f64) -> i32;
type AnalogConfigureFn = unsafe extern "C" fn(i32, i32, i32) -> i32;
type StatusFn = unsafe extern "C" fn(i32, i32, *mut i8) -> i32;
type AnalogStatusData2Fn = unsafe extern "C" fn(i32, i32, *mut f64, i32, i32) -> i32;
type AnalogStatusRecordFn = unsafe extern "C" fn(i32, *mut i32, *mut i32, *mut i32) -> i32;
type AnalogTriggerSourceSetFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogTriggerTypeSetFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogTriggerChannelSetFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogTriggerLevelSetFn = unsafe extern "C" fn(i32, f64) -> i32;
type AnalogTriggerConditionSetFn = unsafe extern "C" fn(i32, i32) -> i32;
type AnalogTriggerAutoTimeoutSetFn = unsafe extern "C" fn(i32, f64) -> i32;
type DigitalClockFn = unsafe extern "C" fn(i32, *mut f64) -> i32;

#[derive(Debug, Serialize, Clone)]
pub struct Ad3Device {
    pub index: i32,
    pub name: String,
    pub serial: String,
    pub revision: i32,
    pub in_use: bool,
    pub configuration_count: i32,
}

#[derive(Debug, Serialize, Clone)]
pub struct Ad3Capabilities {
    pub sdk_version: String,
    pub analog_channels: i32,
    pub analog_bits: i32,
    pub analog_frequency_min: f64,
    pub analog_frequency_max: f64,
    pub analog_buffer_min: i32,
    pub analog_buffer_max: i32,
    pub analog_modes: i32,
    pub digital_bits: i32,
    pub digital_buffer_max: i32,
    pub digital_clock_hz: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AnalogTriggerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_trigger_source")]
    pub source: i32,
    #[serde(default)]
    pub channel: i32,
    #[serde(default)]
    pub level_v: f64,
    #[serde(default)]
    pub slope: i32,
    #[serde(default)]
    pub auto_timeout_s: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AnalogConfig {
    pub channels: Vec<i32>,
    pub frequency_hz: f64,
    pub buffer_size: i32,
    pub range_v: f64,
    pub offset_v: f64,
    pub acquisition_mode: i32,
    pub record_length_s: f64,
    #[serde(default = "default_filter")]
    pub filter: i32,
    #[serde(default)]
    pub trigger: Option<AnalogTriggerConfig>,
    #[serde(default)]
    pub coupling: Option<i32>,
    #[serde(default)]
    pub bandwidth_hz: Option<f64>,
    #[serde(default)]
    pub attenuation: Option<f64>,
    #[serde(default)]
    pub impedance_ohm: Option<f64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct AnalogCapture {
    pub sample_rate_hz: f64,
    pub channels: Vec<Vec<f64>>,
    pub samples_lost: i32,
    pub samples_corrupt: i32,
}

struct DeviceSession {
    library: Library,
    handle: i32,
}

fn default_filter() -> i32 {
    FILTER_DECIMATE
}

fn default_trigger_source() -> i32 {
    TRIGSRC_DETECTOR_ANALOG_IN
}

fn validate_analog_config(config: &AnalogConfig) -> Result<(), String> {
    if config.channels.is_empty() || config.channels.iter().any(|channel| *channel < 0) {
        return Err("analog capture requires at least one valid channel".into());
    }
    if !config.frequency_hz.is_finite() || config.frequency_hz <= 0.0 {
        return Err("analog frequency must be a positive finite value".into());
    }
    if config.buffer_size <= 0 {
        return Err("analog buffer size must be positive".into());
    }
    if !config.range_v.is_finite() || config.range_v <= 0.0 {
        return Err("analog range must be a positive finite value".into());
    }
    if config
        .bandwidth_hz
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        return Err("analog bandwidth must be positive and finite when provided".into());
    }
    if config
        .attenuation
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        return Err("analog attenuation must be positive and finite when provided".into());
    }
    if config
        .impedance_ohm
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        return Err("analog impedance must be positive and finite when provided".into());
    }
    if config.coupling.is_some_and(|value| !matches!(value, 0..=1)) {
        return Err("analog coupling must be 0 (DC) or 1 (AC) when provided".into());
    }
    if !config.offset_v.is_finite() {
        return Err("analog offset must be finite".into());
    }
    if config.acquisition_mode < 0 {
        return Err("analog acquisition mode must not be negative".into());
    }
    if !matches!(config.filter, 0..=3) {
        return Err("analog filter must be between 0 and 3".into());
    }
    if !config.record_length_s.is_finite() || config.record_length_s < 0.0 {
        return Err("analog record length must be finite and non-negative".into());
    }
    if let Some(trigger) = &config.trigger {
        if trigger.source < 0 || trigger.channel < 0 {
            return Err("analog trigger source and channel must not be negative".into());
        }
        if !trigger.level_v.is_finite() {
            return Err("analog trigger level must be finite".into());
        }
        if !matches!(trigger.slope, 0..=2) {
            return Err("analog trigger slope must be 0, 1, or 2".into());
        }
        if !trigger.auto_timeout_s.is_finite() || trigger.auto_timeout_s < 0.0 {
            return Err("analog trigger auto timeout must be finite and non-negative".into());
        }
    }
    Ok(())
}

fn session() -> &'static Mutex<Option<DeviceSession>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

#[tauri::command]
pub fn discover_ad3() -> Result<Vec<Ad3Device>, String> {
    let library =
        load_library().map_err(|error| format!("WaveForms runtime unavailable: {error}"))?;
    unsafe {
        let enumerate: Symbol<EnumFn> = library.get(b"FDwfEnum\0").map_err(symbol_error)?;
        let enum_type: Symbol<EnumTypeFn> =
            library.get(b"FDwfEnumDeviceType\0").map_err(symbol_error)?;
        let enum_used: Symbol<EnumUsedFn> = library
            .get(b"FDwfEnumDeviceIsOpened\0")
            .map_err(symbol_error)?;
        let enum_name: Symbol<EnumTextFn> =
            library.get(b"FDwfEnumDeviceName\0").map_err(symbol_error)?;
        let enum_serial: Symbol<EnumTextFn> = library.get(b"FDwfEnumSN\0").map_err(symbol_error)?;
        let error_text: Symbol<ErrorTextFn> = library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        let enum_config: Option<Symbol<EnumConfigFn>> = library.get(b"FDwfEnumConfig\0").ok();
        let mut count = 0;
        check(enumerate(0, &mut count), &error_text, "FDwfEnum")?;
        let mut devices = Vec::new();
        for index in 0..count {
            let (mut id, mut revision, mut used) = (0, 0, 0);
            let (mut name, mut serial) = ([0 as c_char; 32], [0 as c_char; 32]);
            check(
                enum_type(index, &mut id, &mut revision),
                &error_text,
                "FDwfEnumDeviceType",
            )?;
            if id != DEVICE_ID_DISCOVERY_3 {
                continue;
            }
            check(
                enum_used(index, &mut used),
                &error_text,
                "FDwfEnumDeviceIsOpened",
            )?;
            check(
                enum_name(index, name.as_mut_ptr()),
                &error_text,
                "FDwfEnumDeviceName",
            )?;
            check(
                enum_serial(index, serial.as_mut_ptr()),
                &error_text,
                "FDwfEnumSN",
            )?;
            let configuration_count = if let Some(enum_config) = &enum_config {
                let mut count = 0;
                check(
                    enum_config(index, &mut count),
                    &error_text,
                    "FDwfEnumConfig",
                )?;
                count
            } else {
                0
            };
            devices.push(Ad3Device {
                index,
                name: text(&name),
                serial: normalize_serial(&text(&serial)),
                revision,
                in_use: used != 0,
                configuration_count,
            });
        }
        Ok(devices)
    }
}

pub fn open_ad3(index: i32, config: i32) -> Result<Ad3Capabilities, String> {
    close_ad3()?;
    let library =
        load_library().map_err(|error| format!("WaveForms runtime unavailable: {error}"))?;
    unsafe {
        let mut handle = 0;
        let error_text: Symbol<ErrorTextFn> = library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        // Keep enumeration and opening on the same loaded SDK instance. Like
        // dwfpy's process-wide binding, this avoids invalidating the SDK's
        // enumeration table between separate library loads.
        let enumerate: Symbol<EnumFn> = library.get(b"FDwfEnum\0").map_err(symbol_error)?;
        let enum_type: Symbol<EnumTypeFn> =
            library.get(b"FDwfEnumDeviceType\0").map_err(symbol_error)?;
        let mut device_count = 0;
        check(enumerate(0, &mut device_count), &error_text, "FDwfEnum")?;
        if index < 0 || index >= device_count {
            return Err(format!("AD3 device index {index} is no longer available"));
        }
        let (mut device_id, mut _revision) = (0, 0);
        check(
            enum_type(index, &mut device_id, &mut _revision),
            &error_text,
            "FDwfEnumDeviceType",
        )?;
        if device_id != DEVICE_ID_DISCOVERY_3 {
            return Err(format!(
                "device index {index} is type {device_id}, not Analog Discovery 3"
            ));
        }
        if config < 0 {
            let open: Symbol<DeviceOpenDefaultFn> =
                library.get(b"FDwfDeviceOpen\0").map_err(symbol_error)?;
            check(open(index, &mut handle), &error_text, "FDwfDeviceOpen")?;
        } else {
            let open: Symbol<DeviceOpenFn> = library
                .get(b"FDwfDeviceConfigOpen\0")
                .map_err(symbol_error)?;
            check(
                open(index, config, &mut handle),
                &error_text,
                "FDwfDeviceConfigOpen",
            )?;
        }
        if handle == 0 {
            return Err("WaveForms returned an empty AD3 handle".into());
        }
        *session()
            .lock()
            .map_err(|_| "device session lock poisoned")? = Some(DeviceSession { library, handle });
    }
    read_ad3_capabilities()
}

pub fn close_ad3() -> Result<(), String> {
    let mut guard = session()
        .lock()
        .map_err(|_| "device session lock poisoned")?;
    let Some(device) = guard.as_ref() else {
        return Ok(());
    };
    unsafe {
        let error_text: Symbol<ErrorTextFn> = device
            .library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        let reset: Option<Symbol<DeviceResetFn>> = device.library.get(b"FDwfDeviceReset\0").ok();
        if let Some(reset) = reset {
            // dwfpy resets all instrument modules before releasing the device.
            // Keep closing even if reset is not available on an older runtime.
            let _ = reset(device.handle);
        }
        let close: Symbol<DeviceCloseFn> = device
            .library
            .get(b"FDwfDeviceClose\0")
            .map_err(symbol_error)?;
        if close(device.handle) == 0 {
            let mut buffer = [0 as c_char; 512];
            error_text(buffer.as_mut_ptr());
            let message = text(&buffer);
            return Err(format!(
                "FDwfDeviceClose failed{}",
                if message.is_empty() {
                    String::new()
                } else {
                    format!(": {message}")
                }
            ));
        }
    }
    guard.take();
    Ok(())
}

pub fn read_ad3_capabilities() -> Result<Ad3Capabilities, String> {
    with_device(|device| unsafe {
        let version: Symbol<VersionFn> = device
            .library
            .get(b"FDwfGetVersion\0")
            .map_err(symbol_error)?;
        let error_text: Symbol<ErrorTextFn> = device
            .library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        let channel_count: Symbol<AnalogChannelCountFn> = device
            .library
            .get(b"FDwfAnalogInChannelCount\0")
            .map_err(symbol_error)?;
        let bits_info: Symbol<AnalogBitsInfoFn> = device
            .library
            .get(b"FDwfAnalogInBitsInfo\0")
            .map_err(symbol_error)?;
        let freq_info: Symbol<AnalogFrequencyInfoFn> = device
            .library
            .get(b"FDwfAnalogInFrequencyInfo\0")
            .map_err(symbol_error)?;
        let buffer_info: Symbol<AnalogBufferInfoFn> = device
            .library
            .get(b"FDwfAnalogInBufferSizeInfo\0")
            .map_err(symbol_error)?;
        let mode_info: Symbol<AnalogModeInfoFn> = device
            .library
            .get(b"FDwfAnalogInAcquisitionModeInfo\0")
            .map_err(symbol_error)?;
        let digital_bits: Symbol<AnalogBitsInfoFn> = device
            .library
            .get(b"FDwfDigitalInBitsInfo\0")
            .map_err(symbol_error)?;
        let digital_buffer: Symbol<AnalogBitsInfoFn> = device
            .library
            .get(b"FDwfDigitalInBufferSizeInfo\0")
            .map_err(symbol_error)?;
        let digital_clock: Symbol<DigitalClockFn> = device
            .library
            .get(b"FDwfDigitalInInternalClockInfo\0")
            .map_err(symbol_error)?;
        let (
            mut channels,
            mut bits,
            mut freq_min,
            mut freq_max,
            mut buffer_min,
            mut buffer_max,
            mut modes,
            mut dbits,
            mut dbuffer,
            mut dclock,
        ) = (0, 0, 0.0, 0.0, 0, 0, 0, 0, 0, 0.0);
        let mut sdk_version = [0 as c_char; 32];
        check(
            version(sdk_version.as_mut_ptr()),
            &error_text,
            "FDwfGetVersion",
        )?;
        check(
            channel_count(device.handle, &mut channels),
            &error_text,
            "FDwfAnalogInChannelCount",
        )?;
        check(
            bits_info(device.handle, &mut bits),
            &error_text,
            "FDwfAnalogInBitsInfo",
        )?;
        check(
            freq_info(device.handle, &mut freq_min, &mut freq_max),
            &error_text,
            "FDwfAnalogInFrequencyInfo",
        )?;
        check(
            buffer_info(device.handle, &mut buffer_min, &mut buffer_max),
            &error_text,
            "FDwfAnalogInBufferSizeInfo",
        )?;
        check(
            mode_info(device.handle, &mut modes),
            &error_text,
            "FDwfAnalogInAcquisitionModeInfo",
        )?;
        check(
            digital_bits(device.handle, &mut dbits),
            &error_text,
            "FDwfDigitalInBitsInfo",
        )?;
        check(
            digital_buffer(device.handle, &mut dbuffer),
            &error_text,
            "FDwfDigitalInBufferSizeInfo",
        )?;
        check(
            digital_clock(device.handle, &mut dclock),
            &error_text,
            "FDwfDigitalInInternalClockInfo",
        )?;
        Ok(Ad3Capabilities {
            sdk_version: text(&sdk_version),
            analog_channels: channels,
            analog_bits: bits,
            analog_frequency_min: freq_min,
            analog_frequency_max: freq_max,
            analog_buffer_min: buffer_min,
            analog_buffer_max: buffer_max,
            analog_modes: modes,
            digital_bits: dbits,
            digital_buffer_max: dbuffer,
            digital_clock_hz: dclock,
        })
    })
}

pub fn configure_ad3_analog(config: AnalogConfig) -> Result<AnalogConfig, String> {
    validate_analog_config(&config)?;
    with_device(|device| unsafe {
        let error_text: Symbol<ErrorTextFn> = device
            .library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        let channel_count_fn: Symbol<AnalogChannelCountFn> = device
            .library
            .get(b"FDwfAnalogInChannelCount\0")
            .map_err(symbol_error)?;
        let auto: Symbol<DeviceAutoConfigFn> = device
            .library
            .get(b"FDwfDeviceAutoConfigureSet\0")
            .map_err(symbol_error)?;
        let enable: Symbol<AnalogSetChannelFn> = device
            .library
            .get(b"FDwfAnalogInChannelEnableSet\0")
            .map_err(symbol_error)?;
        let range: Symbol<AnalogSetRangeFn> = device
            .library
            .get(b"FDwfAnalogInChannelRangeSet\0")
            .map_err(symbol_error)?;
        let offset: Symbol<AnalogSetOffsetFn> = device
            .library
            .get(b"FDwfAnalogInChannelOffsetSet\0")
            .map_err(symbol_error)?;
        let frequency: Symbol<AnalogSetFrequencyFn> = device
            .library
            .get(b"FDwfAnalogInFrequencySet\0")
            .map_err(symbol_error)?;
        let buffer: Symbol<AnalogSetBufferFn> = device
            .library
            .get(b"FDwfAnalogInBufferSizeSet\0")
            .map_err(symbol_error)?;
        let mode: Symbol<AnalogSetModeFn> = device
            .library
            .get(b"FDwfAnalogInAcquisitionModeSet\0")
            .map_err(symbol_error)?;
        let filter: Symbol<AnalogSetFilterFn> = device
            .library
            .get(b"FDwfAnalogInChannelFilterSet\0")
            .map_err(symbol_error)?;
        let attenuation: Option<Symbol<AnalogSetAttenuationFn>> = device
            .library
            .get(b"FDwfAnalogInChannelAttenuationSet\0")
            .ok();
        let bandwidth: Option<Symbol<AnalogSetBandwidthFn>> = device
            .library
            .get(b"FDwfAnalogInChannelBandwidthSet\0")
            .ok();
        let impedance: Option<Symbol<AnalogSetImpedanceFn>> = device
            .library
            .get(b"FDwfAnalogInChannelImpedanceSet\0")
            .ok();
        let coupling: Option<Symbol<AnalogSetCouplingFn>> =
            device.library.get(b"FDwfAnalogInChannelCouplingSet\0").ok();
        let record_length: Symbol<AnalogRecordLengthFn> = device
            .library
            .get(b"FDwfAnalogInRecordLengthSet\0")
            .map_err(symbol_error)?;
        let trigger_source: Symbol<AnalogTriggerSourceSetFn> = device
            .library
            .get(b"FDwfAnalogInTriggerSourceSet\0")
            .map_err(symbol_error)?;
        let trigger_type: Symbol<AnalogTriggerTypeSetFn> = device
            .library
            .get(b"FDwfAnalogInTriggerTypeSet\0")
            .map_err(symbol_error)?;
        let trigger_channel: Symbol<AnalogTriggerChannelSetFn> = device
            .library
            .get(b"FDwfAnalogInTriggerChannelSet\0")
            .map_err(symbol_error)?;
        let trigger_level: Symbol<AnalogTriggerLevelSetFn> = device
            .library
            .get(b"FDwfAnalogInTriggerLevelSet\0")
            .map_err(symbol_error)?;
        let trigger_condition: Symbol<AnalogTriggerConditionSetFn> = device
            .library
            .get(b"FDwfAnalogInTriggerConditionSet\0")
            .map_err(symbol_error)?;
        let trigger_timeout: Symbol<AnalogTriggerAutoTimeoutSetFn> = device
            .library
            .get(b"FDwfAnalogInTriggerAutoTimeoutSet\0")
            .map_err(symbol_error)?;
        let configure: Symbol<AnalogConfigureFn> = device
            .library
            .get(b"FDwfAnalogInConfigure\0")
            .map_err(symbol_error)?;
        let frequency_get: Symbol<AnalogFrequencyGetFn> = device
            .library
            .get(b"FDwfAnalogInFrequencyGet\0")
            .map_err(symbol_error)?;
        let buffer_get: Symbol<AnalogBufferGetFn> = device
            .library
            .get(b"FDwfAnalogInBufferSizeGet\0")
            .map_err(symbol_error)?;
        let mode_get: Symbol<AnalogModeGetFn> = device
            .library
            .get(b"FDwfAnalogInAcquisitionModeGet\0")
            .map_err(symbol_error)?;
        let record_length_get: Symbol<AnalogRecordLengthGetFn> = device
            .library
            .get(b"FDwfAnalogInRecordLengthGet\0")
            .map_err(symbol_error)?;
        let mut channel_count = 0;
        check(
            channel_count_fn(device.handle, &mut channel_count),
            &error_text,
            "FDwfAnalogInChannelCount",
        )?;
        check(
            auto(device.handle, 0),
            &error_text,
            "FDwfDeviceAutoConfigureSet",
        )?;
        for channel in 0..channel_count {
            check(
                enable(
                    device.handle,
                    channel,
                    i32::from(config.channels.contains(&channel)),
                ),
                &error_text,
                "FDwfAnalogInChannelEnableSet",
            )?;
        }
        if let Some(channel) = config
            .channels
            .iter()
            .find(|channel| **channel >= channel_count)
        {
            return Err(format!(
                "analog channel {channel} is unavailable; device has {channel_count} channels"
            ));
        }
        check(
            range(device.handle, -1, config.range_v),
            &error_text,
            "FDwfAnalogInChannelRangeSet",
        )?;
        check(
            offset(device.handle, -1, config.offset_v),
            &error_text,
            "FDwfAnalogInChannelOffsetSet",
        )?;
        check(
            frequency(device.handle, config.frequency_hz),
            &error_text,
            "FDwfAnalogInFrequencySet",
        )?;
        check(
            buffer(device.handle, config.buffer_size),
            &error_text,
            "FDwfAnalogInBufferSizeSet",
        )?;
        check(
            mode(device.handle, config.acquisition_mode),
            &error_text,
            "FDwfAnalogInAcquisitionModeSet",
        )?;
        check(
            filter(device.handle, -1, config.filter),
            &error_text,
            "FDwfAnalogInChannelFilterSet",
        )?;
        if let Some(value) = config.attenuation {
            let setter =
                attenuation.ok_or("WaveForms runtime does not support analog attenuation")?;
            check(
                setter(device.handle, -1, value),
                &error_text,
                "FDwfAnalogInChannelAttenuationSet",
            )?;
        }
        if let Some(value) = config.bandwidth_hz {
            let setter = bandwidth.ok_or("WaveForms runtime does not support analog bandwidth")?;
            check(
                setter(device.handle, -1, value),
                &error_text,
                "FDwfAnalogInChannelBandwidthSet",
            )?;
        }
        if let Some(value) = config.impedance_ohm {
            let setter = impedance.ok_or("WaveForms runtime does not support analog impedance")?;
            check(
                setter(device.handle, -1, value),
                &error_text,
                "FDwfAnalogInChannelImpedanceSet",
            )?;
        }
        if let Some(value) = config.coupling {
            let setter = coupling.ok_or("WaveForms runtime does not support analog coupling")?;
            check(
                setter(device.handle, -1, value),
                &error_text,
                "FDwfAnalogInChannelCouplingSet",
            )?;
        }
        check(
            record_length(device.handle, config.record_length_s),
            &error_text,
            "FDwfAnalogInRecordLengthSet",
        )?;
        if let Some(trigger) = &config.trigger {
            if trigger.enabled {
                check(
                    trigger_source(device.handle, trigger.source),
                    &error_text,
                    "FDwfAnalogInTriggerSourceSet",
                )?;
                check(
                    trigger_type(device.handle, TRIGTYPE_EDGE),
                    &error_text,
                    "FDwfAnalogInTriggerTypeSet",
                )?;
                if trigger.source == TRIGSRC_DETECTOR_ANALOG_IN {
                    check(
                        trigger_channel(device.handle, trigger.channel),
                        &error_text,
                        "FDwfAnalogInTriggerChannelSet",
                    )?;
                }
                check(
                    trigger_level(device.handle, trigger.level_v),
                    &error_text,
                    "FDwfAnalogInTriggerLevelSet",
                )?;
                check(
                    trigger_condition(device.handle, trigger.slope),
                    &error_text,
                    "FDwfAnalogInTriggerConditionSet",
                )?;
                check(
                    trigger_timeout(device.handle, trigger.auto_timeout_s),
                    &error_text,
                    "FDwfAnalogInTriggerAutoTimeoutSet",
                )?;
            } else {
                check(
                    trigger_source(device.handle, TRIGSRC_NONE),
                    &error_text,
                    "FDwfAnalogInTriggerSourceSet",
                )?;
            }
        } else {
            check(
                trigger_source(device.handle, TRIGSRC_NONE),
                &error_text,
                "FDwfAnalogInTriggerSourceSet",
            )?;
        }
        check(
            configure(device.handle, 1, 0),
            &error_text,
            "FDwfAnalogInConfigure",
        )?;
        let mut applied = config.clone();
        check(
            frequency_get(device.handle, &mut applied.frequency_hz),
            &error_text,
            "FDwfAnalogInFrequencyGet",
        )?;
        check(
            buffer_get(device.handle, &mut applied.buffer_size),
            &error_text,
            "FDwfAnalogInBufferSizeGet",
        )?;
        check(
            mode_get(device.handle, &mut applied.acquisition_mode),
            &error_text,
            "FDwfAnalogInAcquisitionModeGet",
        )?;
        check(
            record_length_get(device.handle, &mut applied.record_length_s),
            &error_text,
            "FDwfAnalogInRecordLengthGet",
        )?;
        Ok(applied)
    })
}

fn with_device<T>(
    operation: impl FnOnce(&DeviceSession) -> Result<T, String>,
) -> Result<T, String> {
    let guard = session()
        .lock()
        .map_err(|_| "device session lock poisoned")?;
    let device = guard.as_ref().ok_or("no Analog Discovery 3 is open")?;
    operation(device)
}

pub fn set_analog_stream_running(running: bool) -> Result<(), String> {
    with_device(|device| unsafe {
        let errors: Symbol<ErrorTextFn> = device
            .library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        let configure: Symbol<AnalogConfigureFn> = device
            .library
            .get(b"FDwfAnalogInConfigure\0")
            .map_err(symbol_error)?;
        check(
            configure(device.handle, 0, i32::from(running)),
            &errors,
            "FDwfAnalogInConfigure",
        )
    })
}

pub fn read_analog_stream(sample_rate_hz: f64) -> Result<AnalogCapture, String> {
    with_device(|device| unsafe {
        let errors: Symbol<ErrorTextFn> = device
            .library
            .get(b"FDwfGetLastErrorMsg\0")
            .map_err(symbol_error)?;
        let status: Symbol<StatusFn> = device
            .library
            .get(b"FDwfAnalogInStatus\0")
            .map_err(symbol_error)?;
        let record: Symbol<AnalogStatusRecordFn> = device
            .library
            .get(b"FDwfAnalogInStatusRecord\0")
            .map_err(symbol_error)?;
        let data: Symbol<AnalogStatusData2Fn> = device
            .library
            .get(b"FDwfAnalogInStatusData2\0")
            .map_err(symbol_error)?;
        let mut state = 0;
        check(
            status(device.handle, 1, &mut state),
            &errors,
            "FDwfAnalogInStatus",
        )?;
        let (mut available, mut lost, mut corrupt) = (0, 0, 0);
        check(
            record(device.handle, &mut available, &mut lost, &mut corrupt),
            &errors,
            "FDwfAnalogInStatusRecord",
        )?;
        if !(0..=2_000_000).contains(&available) {
            return Err("Invalid streaming sample count returned by AD3".into());
        }
        let mut channels = vec![vec![0.0; available as usize]; 2];
        for (channel, values) in channels.iter_mut().enumerate() {
            if available > 0 {
                check(
                    data(
                        device.handle,
                        channel as i32,
                        values.as_mut_ptr(),
                        0,
                        available,
                    ),
                    &errors,
                    "FDwfAnalogInStatusData2",
                )?;
            }
        }
        if state == DWF_STATE_DONE && available == 0 {
            return Err("AD3 unexpectedly ended the continuous stream".into());
        }
        Ok(AnalogCapture {
            sample_rate_hz,
            channels,
            samples_lost: lost.max(0),
            samples_corrupt: corrupt.max(0),
        })
    })
}

fn load_library() -> Result<Library, libloading::Error> {
    #[cfg(target_os = "windows")]
    {
        unsafe { Library::new("dwf.dll") }
    }
    #[cfg(target_os = "macos")]
    {
        unsafe { Library::new("/Library/Frameworks/dwf.framework/dwf") }
    }
    #[cfg(target_os = "linux")]
    {
        unsafe { Library::new("libdwf.so") }
    }
}

fn text(buffer: &[c_char]) -> String {
    unsafe {
        CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

fn normalize_serial(serial: &str) -> String {
    let normalized = serial.trim().to_ascii_uppercase();
    normalized
        .strip_prefix("SN:")
        .unwrap_or(&normalized)
        .to_string()
}

fn symbol_error(error: libloading::Error) -> String {
    format!("missing WaveForms symbol: {error}")
}

unsafe fn check(
    result: i32,
    error_text: &Symbol<ErrorTextFn>,
    operation: &str,
) -> Result<(), String> {
    if result != 0 {
        return Ok(());
    }
    let mut buffer = [0 as c_char; 512];
    error_text(buffer.as_mut_ptr());
    let message = text(&buffer);
    Err(format!(
        "{operation} failed{}",
        if message.is_empty() {
            String::new()
        } else {
            format!(": {message}")
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_analog_configuration() {
        let mut config = AnalogConfig {
            channels: vec![0],
            frequency_hz: 1_000_000.0,
            buffer_size: 8192,
            range_v: 5.0,
            offset_v: 0.0,
            acquisition_mode: 0,
            record_length_s: 0.0,
            filter: FILTER_DECIMATE,
            trigger: None,
            coupling: None,
            bandwidth_hz: None,
            attenuation: None,
            impedance_ohm: None,
        };
        assert!(validate_analog_config(&config).is_ok());

        config.frequency_hz = 0.0;
        assert!(validate_analog_config(&config).is_err());

        config.frequency_hz = 1_000_000.0;
        config.filter = 4;
        assert!(validate_analog_config(&config).is_err());
    }
}
