use crate::types::CheckReport;

pub fn print_report(report: &CheckReport) {
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
    println!("  capture_mode={:?}", report.capture_mode);
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
