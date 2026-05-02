use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use serde::Serialize;
use wasapi::{
    AudioClientProperties, Device, DeviceEnumerator, Direction, SampleType, StreamCategory,
    StreamMode, StreamOption, WaveFormat, calculate_period_100ns,
};

const DEFAULT_DEVICE_QUERY: &str = "Pico 2 UAC1 Loopback";
const CHANNELS: usize = 2;
const CHANNEL_MASK_STEREO: u32 = 0x3;
const DEFAULT_PERIOD_MS: f64 = 10.0;
const DEFAULT_BUFFER_PERIODS: u32 = 4;

#[derive(Parser)]
#[command(
    author,
    version,
    about = "WASAPI exclusive bit-perfect checker for Pico 2 UAC1 loopback"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List active Windows audio endpoints.
    List,
    /// Test one exact PCM format in WASAPI exclusive polling mode.
    Test(TestArgs),
    /// Test all firmware formats.
    Matrix(MatrixArgs),
}

#[derive(clap::Args, Clone, Debug)]
struct TestArgs {
    #[arg(long, default_value_t = 48_000)]
    rate: u32,

    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u16).range(16..=24))]
    bits: u16,

    #[arg(long, default_value_t = 3.0)]
    seconds: f64,

    /// Substring matched against friendly name, interface name, description, or endpoint id.
    #[arg(long, default_value = DEFAULT_DEVICE_QUERY)]
    device: String,

    /// Exact WASAPI render endpoint id. Overrides --device for playback.
    #[arg(long)]
    render_id: Option<String>,

    /// Exact WASAPI capture endpoint id. Overrides --device for recording.
    #[arg(long)]
    capture_id: Option<String>,

    /// Polling period requested from WASAPI exclusive mode.
    #[arg(long, default_value_t = DEFAULT_PERIOD_MS)]
    period_ms: f64,

    /// Device buffer duration as a multiple of period_ms.
    #[arg(long, default_value_t = DEFAULT_BUFFER_PERIODS)]
    buffer_periods: u32,

    /// Start capture this many ms before playback.
    #[arg(long, default_value_t = 250)]
    pre_roll_ms: u64,

    /// Keep capture running this many ms after playback drain.
    #[arg(long, default_value_t = 500)]
    tail_ms: u64,

    /// Save expected/captured raw data, WAVs, and report.json under this directory.
    #[arg(long)]
    dump_dir: Option<PathBuf>,
}

#[derive(clap::Args, Clone, Debug)]
struct MatrixArgs {
    #[arg(long, default_value_t = 3.0)]
    seconds: f64,

    #[arg(long, default_value = DEFAULT_DEVICE_QUERY)]
    device: String,

    #[arg(long)]
    render_id: Option<String>,

    #[arg(long)]
    capture_id: Option<String>,

    #[arg(long)]
    dump_dir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct AudioConfig {
    rate: u32,
    bits: u16,
    seconds: f64,
    period_ms: f64,
    buffer_periods: u32,
    pre_roll_ms: u64,
    tail_ms: u64,
}

impl AudioConfig {
    fn bytes_per_sample(&self) -> usize {
        usize::from(self.bits / 8)
    }

    fn bytes_per_frame(&self) -> usize {
        self.bytes_per_sample() * CHANNELS
    }

    fn payload_frames(&self) -> usize {
        (self.rate as f64 * self.seconds).round() as usize
    }
}

#[derive(Clone, Debug)]
struct DeviceSelector {
    query: String,
    render_id: Option<String>,
    capture_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct DeviceSummary {
    direction: String,
    id: String,
    friendly_name: String,
    interface_name: String,
    description: String,
    device_format: Option<String>,
}

struct OpenedStream {
    client: wasapi::AudioClient,
    block_align: usize,
    buffer_frames: u32,
    period_hns: i64,
}

#[derive(Debug, Serialize)]
struct StreamStats {
    frames: usize,
    discontinuities: usize,
    silent_buffers: usize,
    timestamp_errors: usize,
    buffer_frames: u32,
    period_hns: i64,
}

#[derive(Debug, Serialize)]
struct CheckReport {
    rate: u32,
    bits: u16,
    seconds: f64,
    exact: bool,
    sync_found: bool,
    latency_frames: Option<usize>,
    latency_ms: Option<f64>,
    compared_bytes: usize,
    expected_bytes: usize,
    captured_bytes: usize,
    mismatched_bytes: usize,
    missing_bytes: usize,
    first_mismatch: Option<Mismatch>,
    capture_stats: StreamStats,
    render_stats: StreamStats,
}

#[derive(Debug, Serialize)]
struct Mismatch {
    byte_offset: usize,
    expected: u8,
    actual: u8,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::List => {
            wasapi::initialize_mta().ok()?;
            list_command()
        }
        Command::Test(args) => {
            let selector = DeviceSelector {
                query: args.device.clone(),
                render_id: args.render_id.clone(),
                capture_id: args.capture_id.clone(),
            };
            let config = AudioConfig {
                rate: args.rate,
                bits: args.bits,
                seconds: args.seconds,
                period_ms: args.period_ms,
                buffer_periods: args.buffer_periods,
                pre_roll_ms: args.pre_roll_ms,
                tail_ms: args.tail_ms,
            };
            validate_config(&config)?;
            let report = run_one(config, selector, args.dump_dir.as_deref())?;
            print_report(&report);
            if report.exact {
                Ok(())
            } else {
                bail!("loopback was not bit-perfect")
            }
        }
        Command::Matrix(args) => {
            let selector = DeviceSelector {
                query: args.device.clone(),
                render_id: args.render_id.clone(),
                capture_id: args.capture_id.clone(),
            };
            let mut failures = 0;
            for bits in [16u16, 24] {
                for rate in [44_100u32, 48_000, 88_200, 96_000] {
                    let config = AudioConfig {
                        rate,
                        bits,
                        seconds: args.seconds,
                        period_ms: DEFAULT_PERIOD_MS,
                        buffer_periods: DEFAULT_BUFFER_PERIODS,
                        pre_roll_ms: 250,
                        tail_ms: 500,
                    };
                    let dump_dir = args
                        .dump_dir
                        .as_ref()
                        .map(|base| base.join(format!("{rate}hz-{bits}bit")));
                    match run_one(config, selector.clone(), dump_dir.as_deref()) {
                        Ok(report) if report.exact => {
                            println!("{rate:>6} Hz {bits:>2}-bit: ok");
                        }
                        Ok(report) => {
                            failures += 1;
                            println!(
                                "{rate:>6} Hz {bits:>2}-bit: FAIL mismatches={} missing={}",
                                report.mismatched_bytes, report.missing_bytes
                            );
                        }
                        Err(err) => {
                            failures += 1;
                            println!("{rate:>6} Hz {bits:>2}-bit: ERROR {err:#}");
                        }
                    }
                }
            }
            if failures == 0 {
                Ok(())
            } else {
                bail!("{failures} matrix cases failed")
            }
        }
    }
}

fn validate_config(config: &AudioConfig) -> Result<()> {
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

fn list_command() -> Result<()> {
    for direction in [Direction::Render, Direction::Capture] {
        println!("{direction:?}:");
        for (index, device) in list_devices(direction)?.iter().enumerate() {
            println!("  [{index}] {}", device.friendly_name);
            println!("      id: {}", device.id);
            println!("      interface: {}", device.interface_name);
            println!("      description: {}", device.description);
            if let Some(format) = &device.device_format {
                println!("      shared format: {format}");
            }
        }
    }
    Ok(())
}

fn run_one(
    config: AudioConfig,
    selector: DeviceSelector,
    dump_dir: Option<&Path>,
) -> Result<CheckReport> {
    let expected = generate_pattern(&config);
    let expected = Arc::new(expected);
    let stop_capture = Arc::new(AtomicBool::new(false));
    let (capture_started_tx, capture_started_rx) = mpsc::sync_channel(1);

    let capture_selector = selector.clone();
    let capture_config = config.clone();
    let capture_stop = Arc::clone(&stop_capture);
    let capture_thread = thread::Builder::new()
        .name("loopback-capture".to_owned())
        .spawn(move || {
            capture_loop(
                capture_config,
                capture_selector,
                capture_stop,
                capture_started_tx,
            )
        })
        .context("spawn capture thread")?;

    capture_started_rx
        .recv_timeout(Duration::from_secs(5))
        .context("capture stream did not start")??;

    thread::sleep(Duration::from_millis(config.pre_roll_ms));

    let render_selector = selector.clone();
    let render_config = config.clone();
    let render_expected = Arc::clone(&expected);
    let render_thread = thread::Builder::new()
        .name("loopback-render".to_owned())
        .spawn(move || render_loop(render_config, render_selector, render_expected))
        .context("spawn render thread")?;

    let render_stats = join_thread(render_thread, "render")?;
    thread::sleep(Duration::from_millis(config.tail_ms));
    stop_capture.store(true, Ordering::SeqCst);
    let (captured, capture_stats) = join_thread(capture_thread, "capture")?;

    let report = analyze(&config, &expected, &captured, capture_stats, render_stats);
    if let Some(dir) = dump_dir {
        write_dumps(dir, &config, &expected, &captured, &report)?;
    }
    Ok(report)
}

fn join_thread<T>(handle: thread::JoinHandle<Result<T>>, name: &str) -> Result<T> {
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(anyhow!("{name} thread panicked")),
    }
}

fn render_loop(
    config: AudioConfig,
    selector: DeviceSelector,
    expected: Arc<Vec<u8>>,
) -> Result<StreamStats> {
    wasapi::initialize_mta().ok()?;
    let opened = open_stream(Direction::Render, &selector, &config)?;
    let render_client = opened.client.get_audiorenderclient()?;
    let mut stats = StreamStats {
        frames: 0,
        discontinuities: 0,
        silent_buffers: 0,
        timestamp_errors: 0,
        buffer_frames: opened.buffer_frames,
        period_hns: opened.period_hns,
    };
    let sleep = polling_sleep(opened.period_hns);
    let tail_frames = frames_from_ms(config.rate, config.tail_ms) + opened.buffer_frames as usize;
    let total_frames = config.payload_frames() + tail_frames;
    let mut frame_offset = 0usize;

    while frame_offset < opened.buffer_frames as usize && frame_offset < total_frames {
        let frames = (opened.buffer_frames as usize)
            .min(total_frames - frame_offset)
            .min(opened.buffer_frames as usize - frame_offset);
        write_render_frames(
            &render_client,
            &expected,
            config.bytes_per_frame(),
            frame_offset,
            frames,
        )?;
        frame_offset += frames;
        stats.frames += frames;
    }

    opened.client.start_stream()?;
    while frame_offset < total_frames {
        let available = opened.client.get_available_space_in_frames()? as usize;
        if available == 0 {
            thread::sleep(sleep);
            continue;
        }
        let frames = available.min(total_frames - frame_offset);
        write_render_frames(
            &render_client,
            &expected,
            config.bytes_per_frame(),
            frame_offset,
            frames,
        )?;
        frame_offset += frames;
        stats.frames += frames;
        thread::sleep(sleep);
    }
    thread::sleep(Duration::from_millis(config.tail_ms));
    opened.client.stop_stream()?;
    Ok(stats)
}

fn capture_loop(
    config: AudioConfig,
    selector: DeviceSelector,
    stop: Arc<AtomicBool>,
    started: mpsc::SyncSender<Result<()>>,
) -> Result<(Vec<u8>, StreamStats)> {
    wasapi::initialize_mta().ok()?;
    let opened = match open_stream(Direction::Capture, &selector, &config) {
        Ok(opened) => opened,
        Err(err) => {
            let _ = started.send(Err(anyhow!("{err:#}")));
            return Err(err);
        }
    };
    let capture_client = opened.client.get_audiocaptureclient()?;
    let mut stats = StreamStats {
        frames: 0,
        discontinuities: 0,
        silent_buffers: 0,
        timestamp_errors: 0,
        buffer_frames: opened.buffer_frames,
        period_hns: opened.period_hns,
    };
    let mut captured = Vec::with_capacity(
        (config.payload_frames()
            + frames_from_ms(config.rate, config.pre_roll_ms + config.tail_ms + 1000))
            * config.bytes_per_frame(),
    );
    let mut scratch = vec![0u8; opened.buffer_frames as usize * opened.block_align];
    let sleep = polling_sleep(opened.period_hns);

    opened.client.start_stream()?;
    let _ = started.send(Ok(()));
    while !stop.load(Ordering::SeqCst) {
        let padding = opened.client.get_current_padding()? as usize;
        if padding == 0 {
            thread::sleep(sleep);
            continue;
        }
        let frames_to_read = padding.min(opened.buffer_frames as usize);
        let bytes_to_read = frames_to_read * opened.block_align;
        let (frames_read, info) = capture_client.read_from_device(&mut scratch[..bytes_to_read])?;
        let bytes_read = frames_read as usize * opened.block_align;
        if info.flags.data_discontinuity {
            stats.discontinuities += 1;
        }
        if info.flags.timestamp_error {
            stats.timestamp_errors += 1;
        }
        if info.flags.silent {
            stats.silent_buffers += 1;
            let old_len = captured.len();
            captured.resize(old_len + bytes_read, 0);
        } else {
            captured.extend_from_slice(&scratch[..bytes_read]);
        }
        stats.frames += frames_read as usize;
    }
    opened.client.stop_stream()?;
    Ok((captured, stats))
}

fn open_stream(
    direction: Direction,
    selector: &DeviceSelector,
    config: &AudioConfig,
) -> Result<OpenedStream> {
    let device = select_device(direction, selector)?;
    let mut client = device.get_iaudioclient()?;
    let requested = WaveFormat::new(
        config.bits as usize,
        config.bits as usize,
        &SampleType::Int,
        config.rate as usize,
        CHANNELS,
        Some(CHANNEL_MASK_STEREO),
    );
    let format = client
        .is_supported_exclusive_with_quirks(&requested)
        .with_context(|| {
            format!(
                "{direction:?} endpoint does not support exact exclusive {} Hz {}-bit stereo",
                config.rate, config.bits
            )
        })?;
    verify_exact_format(&format, config, direction)?;

    let properties = AudioClientProperties::new()
        .set_category(StreamCategory::Media)
        .set_option(StreamOption::Raw)
        .set_option(StreamOption::MatchFormat);
    let _ = client.set_properties(properties);

    let desired_period_hns = (config.period_ms * 10_000.0).round() as i64;
    let period_hns = client
        .calculate_aligned_period_near(desired_period_hns, None, &format)
        .unwrap_or_else(|_| {
            calculate_period_100ns(
                ((config.rate as f64 * config.period_ms / 1000.0).round() as i64).max(1),
                config.rate as i64,
            )
        });
    let buffer_duration_hns = period_hns * i64::from(config.buffer_periods);
    let mode = StreamMode::PollingExclusive {
        buffer_duration_hns,
        period_hns,
    };
    client.initialize_client(&format, &direction, &mode)?;
    let buffer_frames = client.get_buffer_size()?;
    Ok(OpenedStream {
        client,
        block_align: format.get_blockalign() as usize,
        buffer_frames,
        period_hns,
    })
}

fn verify_exact_format(
    format: &WaveFormat,
    config: &AudioConfig,
    direction: Direction,
) -> Result<()> {
    if format.get_samplespersec() != config.rate {
        bail!(
            "{direction:?} accepted a different sample rate: requested {}, got {}",
            config.rate,
            format.get_samplespersec()
        );
    }
    if format.get_nchannels() != CHANNELS as u16 {
        bail!(
            "{direction:?} accepted a different channel count: requested {}, got {}",
            CHANNELS,
            format.get_nchannels()
        );
    }
    if format.get_bitspersample() != config.bits {
        bail!(
            "{direction:?} accepted a different container width: requested {}, got {}",
            config.bits,
            format.get_bitspersample()
        );
    }
    let valid_bits = format.get_validbitspersample();
    if valid_bits != 0 && valid_bits != config.bits {
        bail!(
            "{direction:?} accepted a different valid width: requested {}, got {}",
            config.bits,
            valid_bits
        );
    }
    if format.get_subformat()? != SampleType::Int {
        bail!("{direction:?} accepted a non-integer PCM format");
    }
    Ok(())
}

fn write_render_frames(
    render_client: &wasapi::AudioRenderClient,
    expected: &[u8],
    bytes_per_frame: usize,
    frame_offset: usize,
    frames: usize,
) -> Result<()> {
    let byte_offset = frame_offset * bytes_per_frame;
    let byte_len = frames * bytes_per_frame;
    let mut chunk = vec![0u8; byte_len];
    if byte_offset < expected.len() {
        let available = (expected.len() - byte_offset).min(byte_len);
        chunk[..available].copy_from_slice(&expected[byte_offset..byte_offset + available]);
    }
    render_client.write_to_device(frames, &chunk, None)?;
    Ok(())
}

fn select_device(direction: Direction, selector: &DeviceSelector) -> Result<Device> {
    let enumerator = DeviceEnumerator::new()?;
    let id = match direction {
        Direction::Render => selector.render_id.as_ref(),
        Direction::Capture => selector.capture_id.as_ref(),
    };
    if let Some(id) = id {
        return enumerator
            .get_device(id)
            .with_context(|| format!("open {direction:?} endpoint id {id}"));
    }

    let summaries = list_devices(direction)?;
    let query = selector.query.to_ascii_lowercase();
    let matches: Vec<_> = summaries
        .iter()
        .filter(|device| {
            let haystack = format!(
                "{}\n{}\n{}\n{}",
                device.id, device.friendly_name, device.interface_name, device.description
            )
            .to_ascii_lowercase();
            haystack.contains(&query)
        })
        .collect();

    match matches.as_slice() {
        [device] => enumerator
            .get_device(&device.id)
            .with_context(|| format!("open selected {direction:?} endpoint")),
        [] => {
            let names = summaries
                .iter()
                .map(|device| format!("  {} ({})", device.friendly_name, device.id))
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "no {direction:?} endpoint matched {:?}\n{}",
                selector.query,
                names
            )
        }
        many => {
            let names = many
                .iter()
                .map(|device| format!("  {} ({})", device.friendly_name, device.id))
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "multiple {direction:?} endpoints matched {:?}; pass --{}-id\n{}",
                selector.query,
                if direction == Direction::Render {
                    "render"
                } else {
                    "capture"
                },
                names
            )
        }
    }
}

fn list_devices(direction: Direction) -> Result<Vec<DeviceSummary>> {
    let enumerator = DeviceEnumerator::new()?;
    let collection = enumerator.get_device_collection(&direction)?;
    let mut devices = Vec::new();
    for device in &collection {
        let device = device?;
        let format = device
            .get_device_format()
            .ok()
            .map(|format| describe_format(&format));
        devices.push(DeviceSummary {
            direction: format!("{direction:?}"),
            id: device.get_id()?,
            friendly_name: device.get_friendlyname()?,
            interface_name: device.get_interface_friendlyname().unwrap_or_default(),
            description: device.get_description().unwrap_or_default(),
            device_format: format,
        });
    }
    Ok(devices)
}

fn describe_format(format: &WaveFormat) -> String {
    let sample_type = format
        .get_subformat()
        .map(|ty| ty.to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    format!(
        "{} Hz, {} ch, {} valid / {} container bits, {}, block {}",
        format.get_samplespersec(),
        format.get_nchannels(),
        format.get_validbitspersample(),
        format.get_bitspersample(),
        sample_type,
        format.get_blockalign()
    )
}

fn generate_pattern(config: &AudioConfig) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(config.payload_frames() * config.bytes_per_frame());
    for frame in 0..config.payload_frames() as u32 {
        let left = sample_value(frame, 0, config.bits);
        let right = sample_value(frame, 1, config.bits);
        push_sample(&mut bytes, left, config.bits);
        push_sample(&mut bytes, right, config.bits);
    }
    bytes
}

fn sample_value(frame: u32, channel: u32, bits: u16) -> i32 {
    let mut x = frame
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(channel.wrapping_mul(0x7f4a_7c15))
        .wrapping_add(0x1357_2468);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;

    match bits {
        16 => {
            let value = (x & 0xffff) as i32 - 0x8000;
            value.saturating_mul(3) / 4
        }
        24 => {
            let value = (x & 0x00ff_ffff) as i32 - 0x0080_0000;
            value.saturating_mul(3) / 4
        }
        _ => 0,
    }
}

fn push_sample(bytes: &mut Vec<u8>, sample: i32, bits: u16) {
    match bits {
        16 => bytes.extend_from_slice(&(sample as i16).to_le_bytes()),
        24 => {
            let sample = sample as u32;
            bytes.push(sample as u8);
            bytes.push((sample >> 8) as u8);
            bytes.push((sample >> 16) as u8);
        }
        _ => {}
    }
}

fn analyze(
    config: &AudioConfig,
    expected: &[u8],
    captured: &[u8],
    capture_stats: StreamStats,
    render_stats: StreamStats,
) -> CheckReport {
    let sync_len = expected
        .len()
        .min(config.bytes_per_frame() * (config.rate as usize / 10).max(256));
    let sync_found_at = find_subslice(captured, &expected[..sync_len]);
    let (compared_bytes, mismatched_bytes, missing_bytes, first_mismatch) =
        if let Some(offset) = sync_found_at {
            compare_at_offset(expected, captured, offset)
        } else {
            (0, expected.len(), expected.len(), None)
        };
    let latency_frames = sync_found_at.map(|offset| offset / config.bytes_per_frame());
    let exact = sync_found_at
        .map(|offset| offset % config.bytes_per_frame() == 0)
        .unwrap_or(false)
        && mismatched_bytes == 0
        && missing_bytes == 0;

    CheckReport {
        rate: config.rate,
        bits: config.bits,
        seconds: config.seconds,
        exact,
        sync_found: sync_found_at.is_some(),
        latency_frames,
        latency_ms: latency_frames.map(|frames| frames as f64 * 1000.0 / config.rate as f64),
        compared_bytes,
        expected_bytes: expected.len(),
        captured_bytes: captured.len(),
        mismatched_bytes,
        missing_bytes,
        first_mismatch,
        capture_stats,
        render_stats,
    }
}

fn compare_at_offset(
    expected: &[u8],
    captured: &[u8],
    offset: usize,
) -> (usize, usize, usize, Option<Mismatch>) {
    let available = captured.len().saturating_sub(offset);
    let compared = expected.len().min(available);
    let mut mismatches = 0;
    let mut first = None;
    for index in 0..compared {
        let expected_byte = expected[index];
        let actual_byte = captured[offset + index];
        if expected_byte != actual_byte {
            mismatches += 1;
            if first.is_none() {
                first = Some(Mismatch {
                    byte_offset: index,
                    expected: expected_byte,
                    actual: actual_byte,
                });
            }
        }
    }
    let missing = expected.len().saturating_sub(compared);
    (compared, mismatches, missing, first)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }

    let mut table = vec![0usize; needle.len()];
    let mut len = 0;
    for index in 1..needle.len() {
        while len > 0 && needle[index] != needle[len] {
            len = table[len - 1];
        }
        if needle[index] == needle[len] {
            len += 1;
            table[index] = len;
        }
    }

    let mut matched = 0;
    for (index, byte) in haystack.iter().enumerate() {
        while matched > 0 && *byte != needle[matched] {
            matched = table[matched - 1];
        }
        if *byte == needle[matched] {
            matched += 1;
            if matched == needle.len() {
                return Some(index + 1 - matched);
            }
        }
    }
    None
}

fn write_dumps(
    dir: &Path,
    config: &AudioConfig,
    expected: &[u8],
    captured: &[u8],
    report: &CheckReport,
) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    fs::write(dir.join("expected.raw"), expected)?;
    fs::write(dir.join("captured.raw"), captured)?;
    let mut report_file = File::create(dir.join("report.json"))?;
    serde_json::to_writer_pretty(&mut report_file, report)?;
    report_file.write_all(b"\n")?;
    write_wav(&dir.join("expected.wav"), config, expected)?;
    write_wav(&dir.join("captured.wav"), config, captured)?;
    Ok(())
}

fn write_wav(path: &Path, config: &AudioConfig, bytes: &[u8]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: CHANNELS as u16,
        sample_rate: config.rate,
        bits_per_sample: config.bits,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    match config.bits {
        16 => {
            for sample in bytes.chunks_exact(2) {
                writer.write_sample(i16::from_le_bytes([sample[0], sample[1]]))?;
            }
        }
        24 => {
            for sample in bytes.chunks_exact(3) {
                writer.write_sample(read_i24(sample))?;
            }
        }
        _ => bail!("unsupported WAV bit depth {}", config.bits),
    }
    writer.finalize()?;
    Ok(())
}

fn read_i24(sample: &[u8]) -> i32 {
    let value = (sample[0] as i32) | ((sample[1] as i32) << 8) | ((sample[2] as i32) << 16);
    if value & 0x0080_0000 != 0 {
        value | !0x00ff_ffff
    } else {
        value
    }
}

fn frames_from_ms(rate: u32, millis: u64) -> usize {
    ((u64::from(rate) * millis + 999) / 1000) as usize
}

fn polling_sleep(period_hns: i64) -> Duration {
    let period = Duration::from_nanos((period_hns.max(1) as u64) * 100);
    period / 2
}

fn print_report(report: &CheckReport) {
    println!(
        "{} Hz {}-bit: {}",
        report.rate,
        report.bits,
        if report.exact {
            "bit-perfect"
        } else {
            "FAILED"
        }
    );
    println!(
        "  sync_found={} latency={:?} frames ({:?} ms)",
        report.sync_found, report.latency_frames, report.latency_ms
    );
    println!(
        "  compared={} expected={} captured={} mismatched={} missing={}",
        report.compared_bytes,
        report.expected_bytes,
        report.captured_bytes,
        report.mismatched_bytes,
        report.missing_bytes
    );
    if let Some(first) = &report.first_mismatch {
        println!(
            "  first_mismatch byte={} expected=0x{:02x} actual=0x{:02x}",
            first.byte_offset, first.expected, first.actual
        );
    }
    println!(
        "  capture: frames={} discontinuities={} silent_buffers={} timestamp_errors={}",
        report.capture_stats.frames,
        report.capture_stats.discontinuities,
        report.capture_stats.silent_buffers,
        report.capture_stats.timestamp_errors
    );
}
