use crate::types::{AudioConfig, CheckReport, Mismatch, StreamStats};

pub fn analyze(
    config: &AudioConfig,
    expected: &[u8],
    captured: &[u8],
    capture_stats: StreamStats,
    render_stats: StreamStats,
) -> CheckReport {
    let sync_len = expected
        .len()
        .min(config.bytes_per_frame() * (config.rate as usize / 10).max(256));
    let sync_found_at = find_subslice(captured, &expected[..sync_len])
        .or_else(|| find_short_frame_aligned_sync(config, expected, captured));
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
        capture_mode: config.capture_mode,
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

fn find_short_frame_aligned_sync(
    config: &AudioConfig,
    expected: &[u8],
    captured: &[u8],
) -> Option<usize> {
    let bytes_per_frame = config.bytes_per_frame();
    if bytes_per_frame == 0 || expected.len() < bytes_per_frame || captured.len() < bytes_per_frame
    {
        return None;
    }

    let window_frames = (config.rate as usize / 50).max(64);
    let window_len = expected.len().min(window_frames * bytes_per_frame);
    if captured.len() < window_len {
        return None;
    }

    let prefix_len = window_len.min(bytes_per_frame * 8);
    let max_mismatches = (window_len / 100).max(bytes_per_frame);
    let mut best = None;
    for offset in (0..=captured.len() - window_len).step_by(bytes_per_frame) {
        if captured[offset..offset + prefix_len] != expected[..prefix_len] {
            continue;
        }

        let mut mismatches = 0;
        for index in 0..window_len {
            if captured[offset + index] != expected[index] {
                mismatches += 1;
                if mismatches > max_mismatches {
                    break;
                }
            }
        }
        if mismatches <= max_mismatches
            && best
                .map(|(best_mismatches, _)| mismatches < best_mismatches)
                .unwrap_or(true)
        {
            best = Some((mismatches, offset));
            if mismatches == 0 {
                break;
            }
        }
    }
    best.map(|(_, offset)| offset)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::generate_pattern;
    use crate::types::{CaptureMode, StreamStats, StreamTiming};

    fn config() -> AudioConfig {
        AudioConfig {
            rate: 48_000,
            bits: 24,
            seconds: 1.0,
            timing: StreamTiming::Polling,
            capture_mode: CaptureMode::Exclusive,
            period_ms: 10.0,
            buffer_periods: 4,
            pre_roll_ms: 250,
            tail_ms: 500,
        }
    }

    fn stats() -> StreamStats {
        StreamStats {
            frames: 0,
            discontinuities: 0,
            silent_buffers: 0,
            timestamp_errors: 0,
            buffer_frames: 0,
            period_hns: 0,
        }
    }

    #[test]
    fn short_sync_finds_offset_when_later_payload_has_a_gap() {
        let config = config();
        let expected = generate_pattern(&config);
        let offset = 31 * config.bytes_per_frame();
        let mut captured = vec![0; offset];
        captured.extend_from_slice(&expected);
        captured[offset + config.bytes_per_frame() * 4_000] ^= 0x55;

        let report = analyze(&config, &expected, &captured, stats(), stats());

        assert!(report.sync_found);
        assert_eq!(report.latency_frames, Some(31));
        assert!(!report.exact);
        assert_eq!(report.mismatched_bytes, 1);
    }
}
