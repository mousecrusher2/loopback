use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::types::{AudioConfig, CHANNELS, CheckReport, SampleWidth};

pub fn write_dumps(
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
        sample_rate: config.rate_hz(),
        bits_per_sample: config.bits_per_sample(),
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    match config.sample_width {
        SampleWidth::Bits16 => {
            for sample in bytes.chunks_exact(2) {
                writer.write_sample(i16::from_le_bytes([sample[0], sample[1]]))?;
            }
        }
        SampleWidth::Bits24 => {
            for sample in bytes.chunks_exact(3) {
                writer.write_sample(read_i24(sample))?;
            }
        }
        SampleWidth::Bits32 => {
            for sample in bytes.chunks_exact(4) {
                writer.write_sample(i32::from_le_bytes([
                    sample[0], sample[1], sample[2], sample[3],
                ]))?;
            }
        }
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
