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
