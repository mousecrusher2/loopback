mod analysis;
mod cli;
mod devices;
mod dump;
mod pattern;
mod report;
mod shared_format;
mod stream;
mod types;

use anyhow::{Result, bail};
use clap::Parser;
use std::thread;
use std::time::Duration;

use cli::{CaptureModeArg, Cli, Command, SharedFormatArg, TimingArg};
use devices::{list_command, probe_command};
use report::print_report;
use shared_format::prepare_capture_shared_format;
use stream::run_one;
use types::{
    AudioConfig, AudioRunConfig, CaptureMode, DEFAULT_BUFFER_PERIODS, DEFAULT_PERIOD_MS,
    DeviceSelector, SampleRate, SampleWidth, SharedFormatMode, StreamTiming,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::List => {
            wasapi::initialize_sta().ok()?;
            list_command()
        }
        Command::Probe(args) => {
            wasapi::initialize_sta().ok()?;
            let selector = DeviceSelector {
                query: args.device.clone(),
                render_id: args.render_id.clone(),
                capture_id: args.capture_id.clone(),
            };
            probe_command(&selector)
        }
        Command::Test(args) => {
            let selector = DeviceSelector {
                query: args.device.clone(),
                render_id: args.render_id.clone(),
                capture_id: args.capture_id.clone(),
            };
            let run = AudioRunConfig {
                seconds: args.seconds,
                timing: timing_arg(args.timing),
                capture_mode: capture_mode_arg(args.capture_mode),
                period_ms: args.period_ms,
                buffer_periods: args.buffer_periods,
                pre_roll_ms: args.pre_roll_ms,
                tail_ms: args.tail_ms,
            };
            let config = AudioConfig::from_raw(args.rate, args.bits, run)?;
            let report = run_with_shared_format(
                config,
                selector,
                args.dump_dir.as_deref(),
                shared_format_arg(args.shared_format),
            )?;
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
            let run = AudioRunConfig {
                seconds: args.seconds,
                timing: timing_arg(args.timing),
                capture_mode: capture_mode_arg(args.capture_mode),
                period_ms: DEFAULT_PERIOD_MS,
                buffer_periods: DEFAULT_BUFFER_PERIODS,
                pre_roll_ms: 250,
                tail_ms: 500,
            };
            let mut failures = 0;
            for sample_rate in SampleRate::matrix_order() {
                for sample_width in SampleWidth::matrix_order() {
                    if !sample_width.supports_sample_rate(sample_rate) {
                        continue;
                    }
                    let rate = sample_rate.hz();
                    let bits = sample_width.bits();
                    let config = AudioConfig::new(sample_rate, sample_width, run)?;
                    let dump_dir = args
                        .dump_dir
                        .as_ref()
                        .map(|base| base.join(format!("{rate}hz-{bits}bit")));
                    match run_with_shared_format(
                        config,
                        selector.clone(),
                        dump_dir.as_deref(),
                        shared_format_arg(args.shared_format),
                    ) {
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
                    thread::sleep(Duration::from_millis(5_000));
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

fn run_with_shared_format(
    config: AudioConfig,
    selector: DeviceSelector,
    dump_dir: Option<&std::path::Path>,
    shared_format: SharedFormatMode,
) -> Result<types::CheckReport> {
    let guard = prepare_capture_shared_format(&selector, &config, shared_format)?;
    let result = run_one(config, selector, dump_dir);
    if let Some(guard) = guard {
        let restore_result = guard.restore();
        match (result, restore_result) {
            (Ok(report), Ok(())) => return Ok(report),
            (Ok(_), Err(err)) => return Err(err),
            (Err(err), Ok(())) => return Err(err),
            (Err(err), Err(restore_err)) => {
                return Err(err.context(format!(
                    "also failed to restore capture shared-mode format: {restore_err:#}"
                )));
            }
        }
    }
    result
}

fn timing_arg(arg: TimingArg) -> StreamTiming {
    match arg {
        TimingArg::Polling => StreamTiming::Polling,
        TimingArg::Events => StreamTiming::Events,
    }
}

fn capture_mode_arg(arg: CaptureModeArg) -> CaptureMode {
    match arg {
        CaptureModeArg::Exclusive => CaptureMode::Exclusive,
        CaptureModeArg::Shared => CaptureMode::Shared,
    }
}

fn shared_format_arg(arg: SharedFormatArg) -> SharedFormatMode {
    match arg {
        SharedFormatArg::Leave => SharedFormatMode::Leave,
        SharedFormatArg::SetRestore => SharedFormatMode::SetRestore,
        SharedFormatArg::SetKeep => SharedFormatMode::SetKeep,
    }
}
