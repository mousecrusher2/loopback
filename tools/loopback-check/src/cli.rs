use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::types::{DEFAULT_BUFFER_PERIODS, DEFAULT_DEVICE_QUERY, DEFAULT_PERIOD_MS};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum TimingArg {
    Polling,
    Events,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum CaptureModeArg {
    Exclusive,
    Shared,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum SharedFormatArg {
    /// Leave the Windows shared-mode device format unchanged.
    Leave,
    /// Set the capture endpoint format before the test and restore it afterwards.
    SetRestore,
    /// Set the capture endpoint format and leave it there after the test.
    SetKeep,
}

#[derive(Parser)]
#[command(
    author,
    version,
    about = "WASAPI bit-perfect checker for Pico 2 UAC1 loopback"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List active Windows audio endpoints.
    List,
    /// Activate the selected render and capture endpoints without streaming.
    Probe(ProbeArgs),
    /// Test one exact PCM format.
    Test(TestArgs),
    /// Test all firmware formats.
    Matrix(MatrixArgs),
}

#[derive(clap::Args, Clone, Debug)]
pub struct ProbeArgs {
    /// Substring matched against friendly name, interface name, description, or endpoint id.
    #[arg(long, default_value = DEFAULT_DEVICE_QUERY)]
    pub device: String,

    /// Exact WASAPI render endpoint id. Overrides --device for playback.
    #[arg(long)]
    pub render_id: Option<String>,

    /// Exact WASAPI capture endpoint id. Overrides --device for recording.
    #[arg(long)]
    pub capture_id: Option<String>,
}

#[derive(clap::Args, Clone, Debug)]
pub struct TestArgs {
    #[arg(long, default_value_t = 48_000)]
    pub rate: u32,

    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u16).range(16..=24))]
    pub bits: u16,

    #[arg(long, default_value_t = 3.0)]
    pub seconds: f64,

    /// WASAPI timing mode.
    #[arg(long, value_enum, default_value_t = TimingArg::Polling)]
    pub timing: TimingArg,

    /// WASAPI capture share mode. Render stays exclusive.
    #[arg(long, value_enum, default_value_t = CaptureModeArg::Exclusive)]
    pub capture_mode: CaptureModeArg,

    /// How to handle the Windows shared-mode capture device format.
    #[arg(long, value_enum, default_value_t = SharedFormatArg::SetRestore)]
    pub shared_format: SharedFormatArg,

    /// Substring matched against friendly name, interface name, description, or endpoint id.
    #[arg(long, default_value = DEFAULT_DEVICE_QUERY)]
    pub device: String,

    /// Exact WASAPI render endpoint id. Overrides --device for playback.
    #[arg(long)]
    pub render_id: Option<String>,

    /// Exact WASAPI capture endpoint id. Overrides --device for recording.
    #[arg(long)]
    pub capture_id: Option<String>,

    /// Requested stream period. Shared capture uses this to size its buffer.
    #[arg(long, default_value_t = DEFAULT_PERIOD_MS)]
    pub period_ms: f64,

    /// Device buffer duration as a multiple of period_ms.
    #[arg(long, default_value_t = DEFAULT_BUFFER_PERIODS)]
    pub buffer_periods: u32,

    /// Start capture this many ms before playback.
    #[arg(long, default_value_t = 250)]
    pub pre_roll_ms: u64,

    /// Keep capture running this many ms after playback drain.
    #[arg(long, default_value_t = 500)]
    pub tail_ms: u64,

    /// Save expected/captured raw data, WAVs, and report.json under this directory.
    #[arg(long)]
    pub dump_dir: Option<PathBuf>,
}

#[derive(clap::Args, Clone, Debug)]
pub struct MatrixArgs {
    #[arg(long, default_value_t = 3.0)]
    pub seconds: f64,

    #[arg(long, value_enum, default_value_t = TimingArg::Polling)]
    pub timing: TimingArg,

    /// WASAPI capture share mode. Render stays exclusive.
    #[arg(long, value_enum, default_value_t = CaptureModeArg::Exclusive)]
    pub capture_mode: CaptureModeArg,

    /// How to handle the Windows shared-mode capture device format.
    #[arg(long, value_enum, default_value_t = SharedFormatArg::SetRestore)]
    pub shared_format: SharedFormatArg,

    #[arg(long, default_value = DEFAULT_DEVICE_QUERY)]
    pub device: String,

    #[arg(long)]
    pub render_id: Option<String>,

    #[arg(long)]
    pub capture_id: Option<String>,

    #[arg(long)]
    pub dump_dir: Option<PathBuf>,
}
