#[cfg(feature = "diagnostics")]
mod enabled {
    use core::sync::atomic::{AtomicU32, Ordering};

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
    pub static UAC_DIAG_IN_PIPE_BYTES: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_QUEUE_EMPTY: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_UNDERRUN_BYTES: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_UNDERRUN_PACKETS: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_ALT: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_ALT: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_OUT_RATE: AtomicU32 = AtomicU32::new(0);
    #[unsafe(no_mangle)]
    pub static UAC_DIAG_IN_RATE: AtomicU32 = AtomicU32::new(0);

    pub fn add_out_packet(len: usize) {
        UAC_DIAG_OUT_PACKETS.fetch_add(1, Ordering::Relaxed);
        UAC_DIAG_OUT_BYTES.fetch_add(len as u32, Ordering::Relaxed);
    }

    pub fn add_out_drop() {
        UAC_DIAG_OUT_DROPS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_in_packet(len: usize) {
        UAC_DIAG_IN_PACKETS.fetch_add(1, Ordering::Relaxed);
        UAC_DIAG_IN_BYTES.fetch_add(len as u32, Ordering::Relaxed);
    }

    pub fn add_in_pipe_bytes(len: usize) {
        UAC_DIAG_IN_PIPE_BYTES.fetch_add(len as u32, Ordering::Relaxed);
    }

    pub fn add_in_queue_empty() {
        UAC_DIAG_IN_QUEUE_EMPTY.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_in_underrun(len: usize) {
        UAC_DIAG_IN_UNDERRUN_PACKETS.fetch_add(1, Ordering::Relaxed);
        UAC_DIAG_IN_UNDERRUN_BYTES.fetch_add(len as u32, Ordering::Relaxed);
    }

    pub fn set_out_alt(alt: u8) {
        UAC_DIAG_OUT_ALT.store(u32::from(alt), Ordering::Relaxed);
    }

    pub fn set_in_alt(alt: u8) {
        UAC_DIAG_IN_ALT.store(u32::from(alt), Ordering::Relaxed);
    }

    pub fn set_out_rate(rate: u32) {
        UAC_DIAG_OUT_RATE.store(rate, Ordering::Relaxed);
    }

    pub fn set_in_rate(rate: u32) {
        UAC_DIAG_IN_RATE.store(rate, Ordering::Relaxed);
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
pub fn add_in_pipe_bytes(_len: usize) {}

#[cfg(not(feature = "diagnostics"))]
pub fn add_in_queue_empty() {}

#[cfg(not(feature = "diagnostics"))]
pub fn add_in_underrun(_len: usize) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_out_alt(_alt: u8) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_in_alt(_alt: u8) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_out_rate(_rate: u32) {}

#[cfg(not(feature = "diagnostics"))]
pub fn set_in_rate(_rate: u32) {}
