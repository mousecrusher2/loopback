use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy)]
pub enum InFallbackReason {
    QueueEmpty,
    FormatMismatch,
}

impl InFallbackReason {
    const fn code(self) -> u32 {
        match self {
            Self::QueueEmpty => 1,
            Self::FormatMismatch => 2,
        }
    }
}

pub static UAC_DIAG_IN_FALLBACK_PACKETS: AtomicU32 = AtomicU32::new(0);
pub static UAC_DIAG_IN_FALLBACK_BYTES: AtomicU32 = AtomicU32::new(0);
pub static UAC_DIAG_IN_FALLBACK_LAST_REASON: AtomicU32 = AtomicU32::new(0);

/// Adds one IN fallback packet and its byte length to diagnostics.
///
/// # Panics
///
/// Panics if `len` does not fit in `u32`.
pub fn add_in_fallback(reason: InFallbackReason, len: usize) {
    UAC_DIAG_IN_FALLBACK_PACKETS.fetch_add(1, Ordering::Relaxed);
    let len = u32::try_from(len).unwrap();
    UAC_DIAG_IN_FALLBACK_BYTES.fetch_add(len, Ordering::Relaxed);
    UAC_DIAG_IN_FALLBACK_LAST_REASON.store(reason.code(), Ordering::Relaxed);
}

pub fn in_fallback_packets() -> u32 {
    UAC_DIAG_IN_FALLBACK_PACKETS.load(Ordering::Relaxed)
}
