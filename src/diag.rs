use core::sync::atomic::{AtomicU32, Ordering};

use crate::audio::{AudioStreamingAlternateSetting, SampleRate};

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

// SAFETY: These diagnostics symbols use unique project-prefixed names and are
// defined exactly once. They are atomics, so debugger reads do not require
// non-atomic access. The atomics are kept even without the diagnostics feature
// so the LED task can observe fallback events without exporting the symbols.
#[cfg_attr(feature = "diagnostics", unsafe(no_mangle))]
pub static UAC_DIAG_IN_FALLBACK_PACKETS: AtomicU32 = AtomicU32::new(0);
#[cfg_attr(feature = "diagnostics", unsafe(no_mangle))]
pub static UAC_DIAG_IN_FALLBACK_BYTES: AtomicU32 = AtomicU32::new(0);
#[cfg_attr(feature = "diagnostics", unsafe(no_mangle))]
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

#[cfg(feature = "diagnostics")]
mod enabled {
    use core::sync::atomic::{AtomicU32, Ordering};

    use super::{AudioStreamingAlternateSetting, SampleRate};

    // SAFETY: The exported diagnostics symbols below all use unique
    // project-prefixed names and are defined exactly once in the firmware.
    // They are atomics, so debugger reads do not require non-atomic access.
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_PACKETS: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_BYTES: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_DROPS: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_PACKETS: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_BYTES: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_LOOPBACK_BYTES: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_QUEUE_EMPTY: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_ALT: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_ALT: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_RATE: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_RATE: AtomicU32 = AtomicU32::new(0);

    /// Adds one OUT packet and its byte length to diagnostics.
    ///
    /// # Panics
    ///
    /// Panics if `len` does not fit in `u32`.
    pub fn add_out_packet(len: usize) {
        UAC_DIAG_OUT_PACKETS.fetch_add(1, Ordering::Relaxed);
        let len = u32::try_from(len).unwrap();
        UAC_DIAG_OUT_BYTES.fetch_add(len, Ordering::Relaxed);
    }

    pub fn add_out_drop() {
        UAC_DIAG_OUT_DROPS.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds one IN packet and its byte length to diagnostics.
    ///
    /// # Panics
    ///
    /// Panics if `len` does not fit in `u32`.
    pub fn add_in_packet(len: usize) {
        UAC_DIAG_IN_PACKETS.fetch_add(1, Ordering::Relaxed);
        let len = u32::try_from(len).unwrap();
        UAC_DIAG_IN_BYTES.fetch_add(len, Ordering::Relaxed);
    }

    /// Adds loopback byte length to diagnostics.
    ///
    /// # Panics
    ///
    /// Panics if `len` does not fit in `u32`.
    pub fn add_in_loopback_bytes(len: usize) {
        let len = u32::try_from(len).unwrap();
        UAC_DIAG_IN_LOOPBACK_BYTES.fetch_add(len, Ordering::Relaxed);
    }

    pub fn add_in_queue_empty() {
        UAC_DIAG_IN_QUEUE_EMPTY.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_out_alt(alt: AudioStreamingAlternateSetting) {
        UAC_DIAG_OUT_ALT.store(u32::from(alt.number()), Ordering::Relaxed);
    }

    pub fn set_in_alt(alt: AudioStreamingAlternateSetting) {
        UAC_DIAG_IN_ALT.store(u32::from(alt.number()), Ordering::Relaxed);
    }

    pub fn set_out_rate(rate: Option<SampleRate>) {
        UAC_DIAG_OUT_RATE.store(rate.map_or(0, SampleRate::hz), Ordering::Relaxed);
    }

    pub fn set_in_rate(rate: Option<SampleRate>) {
        UAC_DIAG_IN_RATE.store(rate.map_or(0, SampleRate::hz), Ordering::Relaxed);
    }
}

#[cfg(feature = "diagnostics")]
pub use enabled::*;

#[cfg(not(feature = "diagnostics"))]
pub fn add_out_packet(_len: usize) {}

#[cfg(not(feature = "diagnostics"))]
pub fn add_out_drop() {}

#[cfg(not(feature = "diagnostics"))]
pub fn add_in_packet(_len: usize) {}

#[cfg(not(feature = "diagnostics"))]
pub fn add_in_loopback_bytes(_len: usize) {}

#[cfg(not(feature = "diagnostics"))]
pub fn add_in_queue_empty() {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_out_alt(_alt: AudioStreamingAlternateSetting) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_in_alt(_alt: AudioStreamingAlternateSetting) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_out_rate(_rate: Option<SampleRate>) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_in_rate(_rate: Option<SampleRate>) {}
