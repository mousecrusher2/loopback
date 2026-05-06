use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;

pub const DEFAULT_DEVICE_QUERY: &str = "Pico 2 UAC1 Loopback";
pub const CHANNELS: usize = 2;
pub const CHANNEL_MASK_STEREO: u32 = 0x3;
pub const DEFAULT_PERIOD_MS: f64 = 10.0;
pub const DEFAULT_BUFFER_PERIODS: u32 = 4;

#[derive(Clone, Debug)]
pub struct AudioConfig {
    pub rate: u32,
    pub bits: u16,
    pub seconds: f64,
    pub period_ms: f64,
    pub buffer_periods: u32,
    pub pre_roll_ms: u64,
    pub tail_ms: u64,
}

impl AudioConfig {
    pub fn bytes_per_sample(&self) -> usize {
        usize::from(self.bits / 8)
    }

    pub fn bytes_per_frame(&self) -> usize {
        self.bytes_per_sample() * CHANNELS
    }

    pub fn payload_frames(&self) -> usize {
        (self.rate as f64 * self.seconds).round() as usize
    }
}

#[derive(Clone, Debug)]
pub struct DeviceSelector {
    pub query: String,
    pub render_id: Option<String>,
    pub capture_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceSummary {
    pub direction: String,
    pub id: String,
    pub friendly_name: String,
    pub interface_name: String,
    pub description: String,
    pub device_format: Option<String>,
}

pub struct OpenedStream {
    pub client: wasapi::AudioClient,
    pub block_align: usize,
    pub buffer_frames: u32,
    pub period_hns: i64,
}

#[derive(Debug, Serialize)]
pub struct StreamStats {
    pub frames: usize,
    pub discontinuities: usize,
    pub silent_buffers: usize,
    pub timestamp_errors: usize,
    pub buffer_frames: u32,
    pub period_hns: i64,
}

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub rate: u32,
    pub bits: u16,
    pub seconds: f64,
    pub exact: bool,
    pub sync_found: bool,
    pub latency_frames: Option<usize>,
    pub latency_ms: Option<f64>,
    pub compared_bytes: usize,
    pub expected_bytes: usize,
    pub captured_bytes: usize,
    pub mismatched_bytes: usize,
    pub missing_bytes: usize,
    pub first_mismatch: Option<Mismatch>,
    pub capture_stats: StreamStats,
    pub render_stats: StreamStats,
}

#[derive(Debug, Serialize)]
pub struct Mismatch {
    pub byte_offset: usize,
    pub expected: u8,
    pub actual: u8,
}

pub fn validate_config(config: &AudioConfig) -> Result<()> {
    if !matches!(config.rate, 44_100 | 48_000 | 88_200 | 96_000) {
        bail!("rate must be one of 44100, 48000, 88200, 96000");
    }
    if !matches!(config.bits, 16 | 24) {
        bail!("bits must be 16 or 24");
    }
    if !(0.1..=30.0).contains(&config.seconds) {
        bail!("seconds must be between 0.1 and 30.0");
    }
    if config.period_ms <= 0.0 {
        bail!("period-ms must be positive");
    }
    if config.buffer_periods == 0 {
        bail!("buffer-periods must be positive");
    }
    Ok(())
}

pub fn frames_from_ms(rate: u32, millis: u64) -> usize {
    (u64::from(rate) * millis).div_ceil(1000) as usize
}

pub fn polling_sleep(period_hns: i64) -> Duration {
    let period = Duration::from_nanos((period_hns.max(1) as u64) * 100);
    period / 2
}
