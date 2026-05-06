mod analysis;
mod cli;
mod devices;
mod dump;
mod pattern;
mod report;
mod stream;
mod types;

use anyhow::{Result, bail};
use clap::Parser;

use cli::{Cli, Command};
use devices::list_command;
use report::print_report;
use stream::run_one;
use types::{
    AudioConfig, DEFAULT_BUFFER_PERIODS, DEFAULT_PERIOD_MS, DeviceSelector, validate_config,
};

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
