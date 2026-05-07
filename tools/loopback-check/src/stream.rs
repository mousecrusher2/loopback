use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use wasapi::{
    AudioClientProperties, Direction, SampleType, ShareMode as WasapiShareMode, StreamCategory,
    StreamMode, StreamOption, WaveFormat, calculate_period_100ns,
};

use crate::analysis::analyze;
use crate::devices::select_device;
use crate::dump::write_dumps;
use crate::pattern::generate_pattern;
use crate::types::{
    AudioConfig, CHANNEL_MASK_STEREO, CHANNELS, CaptureMode, CheckReport, DeviceSelector,
    OpenedStream, StreamStats, StreamTiming, frames_from_ms, polling_sleep,
};

pub fn run_one(
    config: AudioConfig,
    selector: DeviceSelector,
    dump_dir: Option<&Path>,
) -> Result<CheckReport> {
    let expected = generate_pattern(&config);
    let expected = Arc::new(expected);
    let stop_capture = Arc::new(AtomicBool::new(false));
    let (render_started_tx, render_started_rx) = mpsc::sync_channel(1);
    let (payload_start_tx, payload_start_rx) = mpsc::sync_channel(1);
    let (capture_started_tx, capture_started_rx) = mpsc::sync_channel(1);

    let render_selector = selector.clone();
    let render_config = config.clone();
    let render_expected = Arc::clone(&expected);
    let render_thread = thread::Builder::new()
        .name("loopback-render".to_owned())
        .spawn(move || {
            render_loop(
                render_config,
                render_selector,
                render_expected,
                render_started_tx,
                payload_start_rx,
            )
        })
        .context("spawn render thread")?;

    render_started_rx
        .recv_timeout(Duration::from_secs(5))
        .context("render stream did not start")??;

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

    if let Err(err) = capture_started_rx
        .recv_timeout(Duration::from_secs(5))
        .context("capture stream did not start")
        .and_then(|result| result)
    {
        drop(payload_start_tx);
        let _ = join_thread(render_thread, "render");
        return Err(err);
    }

    thread::sleep(Duration::from_millis(config.pre_roll_ms));
    payload_start_tx
        .send(())
        .context("signal render payload start")?;

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
    started: mpsc::SyncSender<Result<()>>,
    payload_start: mpsc::Receiver<()>,
) -> Result<StreamStats> {
    wasapi::initialize_sta().ok()?;
    let opened = match open_stream(Direction::Render, &selector, &config) {
        Ok(opened) => opened,
        Err(err) => {
            let _ = started.send(Err(anyhow!("{err:#}")));
            return Err(err);
        }
    };
    let render_client = opened
        .client
        .get_audiorenderclient()
        .context("get WASAPI render client")?;
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

    if opened.buffer_frames > 0 {
        write_silence_frames(
            &render_client,
            opened.buffer_frames as usize,
            opened.block_align,
        )?;
    }

    opened.client.start_stream().context("start render stream")?;
    let _ = started.send(Ok(()));
    loop {
        match payload_start.try_recv() {
            Ok(()) => break,
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => bail!("render payload start was canceled"),
        }
        let available = wait_render_space(&opened, sleep, 100)?;
        if available > 0 {
            write_silence_frames(&render_client, available, opened.block_align)?;
        }
    }

    while frame_offset < total_frames {
        let available = wait_render_space(&opened, sleep, 1_000)?;
        if available == 0 {
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
        if opened.event_handle.is_none() {
            thread::sleep(sleep);
        }
    }
    thread::sleep(Duration::from_millis(config.tail_ms));
    opened.client.stop_stream().context("stop render stream")?;
    Ok(stats)
}

fn wait_render_space(
    opened: &OpenedStream,
    sleep: Duration,
    event_timeout_ms: u32,
) -> Result<usize> {
    if let Some(handle) = &opened.event_handle {
        handle.wait_for_event(event_timeout_ms)?;
    }
    let available = opened.client.get_available_space_in_frames()? as usize;
    if available == 0 && opened.event_handle.is_none() {
        thread::sleep(sleep);
    }
    Ok(available)
}

fn capture_loop(
    config: AudioConfig,
    selector: DeviceSelector,
    stop: Arc<AtomicBool>,
    started: mpsc::SyncSender<Result<()>>,
) -> Result<(Vec<u8>, StreamStats)> {
    wasapi::initialize_sta().ok()?;
    let opened = match open_stream(Direction::Capture, &selector, &config) {
        Ok(opened) => opened,
        Err(err) => {
            let _ = started.send(Err(anyhow!("{err:#}")));
            return Err(err);
        }
    };
    let capture_client = opened
        .client
        .get_audiocaptureclient()
        .context("get WASAPI capture client")?;
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

    opened.client.start_stream().context("start capture stream")?;
    let _ = started.send(Ok(()));
    while !stop.load(Ordering::SeqCst) {
        if let Some(handle) = &opened.event_handle {
            match handle.wait_for_event(1_000) {
                Ok(()) => {}
                Err(_) if stop.load(Ordering::SeqCst) => break,
                Err(err) => return Err(err.into()),
            }
        }
        let padding = opened.client.get_current_padding()? as usize;
        if padding == 0 {
            if opened.event_handle.is_none() {
                thread::sleep(sleep);
            }
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
    opened.client.stop_stream().context("stop capture stream")?;
    Ok((captured, stats))
}

fn open_stream(
    direction: Direction,
    selector: &DeviceSelector,
    config: &AudioConfig,
) -> Result<OpenedStream> {
    let device = select_device(direction, selector)?;
    let mut client = device
        .get_iaudioclient()
        .with_context(|| format!("activate IAudioClient for {direction:?} endpoint"))?;
    let requested = WaveFormat::new(
        config.bits as usize,
        config.bits as usize,
        &SampleType::Int,
        config.rate as usize,
        CHANNELS,
        Some(CHANNEL_MASK_STEREO),
    );
    let capture_mode = if direction == Direction::Capture {
        config.capture_mode
    } else {
        CaptureMode::Exclusive
    };
    let format = match capture_mode {
        CaptureMode::Exclusive => client
            .is_supported_exclusive_with_quirks(&requested)
            .with_context(|| {
                format!(
                    "{direction:?} endpoint does not support exact exclusive {} Hz {}-bit stereo",
                    config.rate, config.bits
                )
            })?,
        CaptureMode::Shared => match client
            .is_supported(&requested, &WasapiShareMode::Shared)
            .with_context(|| {
                format!(
                    "{direction:?} endpoint does not support shared {} Hz {}-bit stereo",
                    config.rate, config.bits
                )
            })? {
            None => requested,
            Some(similar) => bail!(
                "{direction:?} shared mode would use a different format: requested {} Hz {}-bit stereo, nearest is {} Hz {} valid / {} container bits {:?}",
                config.rate,
                config.bits,
                similar.get_samplespersec(),
                similar.get_validbitspersample(),
                similar.get_bitspersample(),
                similar.get_subformat()?
            ),
        },
    };
    verify_exact_format(&format, config, direction)?;

    let properties = AudioClientProperties::new()
        .set_category(StreamCategory::Media)
        .set_option(StreamOption::Raw)
        .set_option(StreamOption::MatchFormat);
    let _ = client.set_properties(properties);

    let desired_period_hns = (config.period_ms * 10_000.0).round() as i64;
    let (mode, period_hns) =
        stream_mode(&mut client, &format, config, capture_mode, desired_period_hns)?;
    client
        .initialize_client(&format, &direction, &mode)
        .with_context(|| {
            format!(
                "initialize {direction:?} {capture_mode:?} {:?} stream: {} Hz {}-bit, period_hns={}",
                config.timing, config.rate, config.bits, period_hns
            )
        })?;
    let event_handle = if config.timing == StreamTiming::Events {
        Some(
            client
                .set_get_eventhandle()
                .with_context(|| format!("set {direction:?} event handle"))?,
        )
    } else {
        None
    };
    let buffer_frames = client
        .get_buffer_size()
        .with_context(|| format!("get {direction:?} endpoint buffer size"))?;
    Ok(OpenedStream {
        client,
        event_handle,
        block_align: format.get_blockalign() as usize,
        buffer_frames,
        period_hns,
    })
}

fn stream_mode(
    client: &mut wasapi::AudioClient,
    format: &WaveFormat,
    config: &AudioConfig,
    capture_mode: CaptureMode,
    desired_period_hns: i64,
) -> Result<(StreamMode, i64)> {
    match capture_mode {
        CaptureMode::Exclusive => {
            let period_hns = client
                .calculate_aligned_period_near(desired_period_hns, None, format)
                .unwrap_or_else(|_| {
                    calculate_period_100ns(
                        ((config.rate as f64 * config.period_ms / 1000.0).round() as i64).max(1),
                        config.rate as i64,
                    )
                });
            let mode = match config.timing {
                StreamTiming::Polling => {
                    let buffer_duration_hns = period_hns * i64::from(config.buffer_periods);
                    StreamMode::PollingExclusive {
                        buffer_duration_hns,
                        period_hns,
                    }
                }
                StreamTiming::Events => StreamMode::EventsExclusive { period_hns },
            };
            Ok((mode, period_hns))
        }
        CaptureMode::Shared => {
            let (default_period_hns, min_period_hns) = client
                .get_device_period()
                .context("get shared-mode capture device period")?;
            let buffer_duration_hns =
                desired_period_hns.max(min_period_hns) * i64::from(config.buffer_periods);
            let mode = match config.timing {
                StreamTiming::Polling => StreamMode::PollingShared {
                    autoconvert: false,
                    buffer_duration_hns,
                },
                StreamTiming::Events => StreamMode::EventsShared {
                    autoconvert: false,
                    buffer_duration_hns,
                },
            };
            Ok((mode, default_period_hns))
        }
    }
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

fn write_silence_frames(
    render_client: &wasapi::AudioRenderClient,
    frames: usize,
    bytes_per_frame: usize,
) -> Result<()> {
    let chunk = vec![0u8; frames * bytes_per_frame];
    render_client.write_to_device(frames, &chunk, None)?;
    Ok(())
}
