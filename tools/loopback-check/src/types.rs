use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;

pub const DEFAULT_DEVICE_QUERY: &str = "Pico 2 UAC1 Loopback";
pub const CHANNELS: usize = 2;
pub const CHANNEL_MASK_STEREO: u32 = 0x3;
pub const DEFAULT_PERIOD_MS: f64 = 10.0;
pub const DEFAULT_BUFFER_PERIODS: u32 = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StreamTiming {
    #[default]
    Polling,
    Events,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum CaptureMode {
    #[default]
    Exclusive,
    Shared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedFormatMode {
    Leave,
    SetRestore,
    SetKeep,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SampleRate {
    R44100,
    #[default]
    R48000,
    R88200,
    R96000,
}

impl SampleRate {
    pub const fn hz(self) -> u32 {
        match self {
            Self::R44100 => 44_100,
            Self::R48000 => 48_000,
            Self::R88200 => 88_200,
            Self::R96000 => 96_000,
        }
    }

    pub fn from_hz(rate: u32) -> Result<Self> {
        match rate {
            44_100 => Ok(Self::R44100),
            48_000 => Ok(Self::R48000),
            88_200 => Ok(Self::R88200),
            96_000 => Ok(Self::R96000),
            _ => bail!("rate must be one of 44100, 48000, 88200, 96000"),
        }
    }

    pub const fn matrix_order() -> [Self; 4] {
        [Self::R96000, Self::R88200, Self::R48000, Self::R44100]
    }

    pub fn frames_from_ms(self, millis: u64) -> usize {
        (u64::from(self.hz()) * millis).div_ceil(1000) as usize
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SampleWidth {
    #[default]
    Bits16,
    Bits24,
    Bits32,
}

impl SampleWidth {
    pub const fn bits(self) -> u16 {
        match self {
            Self::Bits16 => 16,
            Self::Bits24 => 24,
            Self::Bits32 => 32,
        }
    }

    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::Bits16 => 2,
            Self::Bits24 => 3,
            Self::Bits32 => 4,
        }
    }

    pub const fn supports_sample_rate(self, rate: SampleRate) -> bool {
        match self {
            Self::Bits16 | Self::Bits24 => true,
            Self::Bits32 => matches!(rate, SampleRate::R44100 | SampleRate::R48000),
        }
    }

    pub fn from_bits(bits: u16) -> Result<Self> {
        match bits {
            16 => Ok(Self::Bits16),
            24 => Ok(Self::Bits24),
            32 => Ok(Self::Bits32),
            _ => bail!("bits must be 16, 24, or 32"),
        }
    }

    pub const fn matrix_order() -> [Self; 3] {
        [Self::Bits32, Self::Bits24, Self::Bits16]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AudioRunConfig {
    pub seconds: f64,
    pub timing: StreamTiming,
    pub capture_mode: CaptureMode,
    pub period_ms: f64,
    pub buffer_periods: u32,
    pub pre_roll_ms: u64,
    pub tail_ms: u64,
}

impl Default for AudioRunConfig {
    fn default() -> Self {
        Self {
            seconds: 3.0,
            timing: StreamTiming::default(),
            capture_mode: CaptureMode::default(),
            period_ms: DEFAULT_PERIOD_MS,
            buffer_periods: DEFAULT_BUFFER_PERIODS,
            pre_roll_ms: 250,
            tail_ms: 500,
        }
    }
}

impl AudioRunConfig {
    fn validate(self) -> Result<()> {
        if !(0.1..=30.0).contains(&self.seconds) {
            bail!("seconds must be between 0.1 and 30.0");
        }
        if self.period_ms <= 0.0 {
            bail!("period-ms must be positive");
        }
        if self.buffer_periods == 0 {
            bail!("buffer-periods must be positive");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AudioConfig {
    pub sample_rate: SampleRate,
    pub sample_width: SampleWidth,
    pub seconds: f64,
    pub timing: StreamTiming,
    pub capture_mode: CaptureMode,
    pub period_ms: f64,
    pub buffer_periods: u32,
    pub pre_roll_ms: u64,
    pub tail_ms: u64,
}

impl AudioConfig {
    pub fn from_raw(rate: u32, bits: u16, run: AudioRunConfig) -> Result<Self> {
        let sample_rate = SampleRate::from_hz(rate)?;
        let sample_width = SampleWidth::from_bits(bits)?;
        Self::new(sample_rate, sample_width, run)
    }

    pub fn new(
        sample_rate: SampleRate,
        sample_width: SampleWidth,
        run: AudioRunConfig,
    ) -> Result<Self> {
        if !sample_width.supports_sample_rate(sample_rate) {
            bail!("32-bit checks are limited to 44100 or 48000");
        }
        run.validate()?;
        Ok(Self {
            sample_rate,
            sample_width,
            seconds: run.seconds,
            timing: run.timing,
            capture_mode: run.capture_mode,
            period_ms: run.period_ms,
            buffer_periods: run.buffer_periods,
            pre_roll_ms: run.pre_roll_ms,
            tail_ms: run.tail_ms,
        })
    }

    pub const fn rate_hz(&self) -> u32 {
        self.sample_rate.hz()
    }

    pub const fn bits_per_sample(&self) -> u16 {
        self.sample_width.bits()
    }

    pub const fn bytes_per_sample(&self) -> usize {
        self.sample_width.bytes_per_sample()
    }

    pub fn bytes_per_frame(&self) -> usize {
        self.bytes_per_sample() * CHANNELS
    }

    pub fn payload_frames(&self) -> usize {
        (self.rate_hz() as f64 * self.seconds).round() as usize
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
    pub event_handle: Option<wasapi::Handle>,
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
    pub capture_mode: CaptureMode,
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

pub fn polling_sleep(period_hns: i64) -> Duration {
    let period = Duration::from_nanos((period_hns.max(1) as u64) * 100);
    period / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_unsupported_32_bit_rate() {
        let err = AudioConfig::new(
            SampleRate::R96000,
            SampleWidth::Bits32,
            AudioRunConfig::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("32-bit"));
    }

    #[test]
    fn raw_config_rejects_unknown_width() {
        let err = AudioConfig::from_raw(48_000, 20, AudioRunConfig::default()).unwrap_err();

        assert!(err.to_string().contains("bits"));
    }
}
