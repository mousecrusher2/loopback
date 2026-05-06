use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

pub const CHANNEL_COUNT: u8 = 2;
pub const SAMPLE_WIDTH_16_ALT: u8 = 1;
pub const SAMPLE_WIDTH_24_ALT: u8 = 2;
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
pub const SUPPORTED_SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
pub const MAX_PACKET_SIZE_16: usize = 96_000 / 1_000 * CHANNEL_COUNT as usize * 2;
pub const MAX_PACKET_SIZE_24: usize = 96_000 / 1_000 * CHANNEL_COUNT as usize * 3;
pub const MAX_PACKET_SIZE: usize = MAX_PACKET_SIZE_24;
pub const PIPE_SIZE: usize = MAX_PACKET_SIZE * 8;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum StreamDirection {
    Out,
    In,
}

pub struct AudioState {
    out_alt: AtomicU8,
    in_alt: AtomicU8,
    out_rate_hz: AtomicU32,
    in_rate_hz: AtomicU32,
}

impl AudioState {
    pub const fn new() -> Self {
        Self {
            out_alt: AtomicU8::new(0),
            in_alt: AtomicU8::new(0),
            out_rate_hz: AtomicU32::new(DEFAULT_SAMPLE_RATE),
            in_rate_hz: AtomicU32::new(DEFAULT_SAMPLE_RATE),
        }
    }

    pub fn reset(&self) {
        self.out_alt.store(0, Ordering::Relaxed);
        self.in_alt.store(0, Ordering::Relaxed);
        self.out_rate_hz
            .store(DEFAULT_SAMPLE_RATE, Ordering::Relaxed);
        self.in_rate_hz
            .store(DEFAULT_SAMPLE_RATE, Ordering::Relaxed);
    }

    pub fn set_alt(&self, direction: StreamDirection, alternate_setting: u8) {
        match direction {
            StreamDirection::Out => self.out_alt.store(alternate_setting, Ordering::Relaxed),
            StreamDirection::In => self.in_alt.store(alternate_setting, Ordering::Relaxed),
        }
    }

    pub fn set_rate_hz(&self, direction: StreamDirection, rate_hz: u32) {
        match direction {
            StreamDirection::Out => self.out_rate_hz.store(rate_hz, Ordering::Relaxed),
            StreamDirection::In => self.in_rate_hz.store(rate_hz, Ordering::Relaxed),
        }
    }

    pub fn rate_hz(&self, direction: StreamDirection) -> u32 {
        match direction {
            StreamDirection::Out => self.out_rate_hz.load(Ordering::Relaxed),
            StreamDirection::In => self.in_rate_hz.load(Ordering::Relaxed),
        }
    }

    pub fn in_bytes_per_audio_frame(&self) -> usize {
        bytes_per_audio_frame(self.in_alt.load(Ordering::Relaxed))
    }

    pub fn out_bytes_per_audio_frame(&self) -> usize {
        bytes_per_audio_frame(self.out_alt.load(Ordering::Relaxed))
    }

    pub fn loopback_format_matches(&self) -> bool {
        let out_alt = self.out_alt.load(Ordering::Relaxed);
        let in_alt = self.in_alt.load(Ordering::Relaxed);

        out_alt != 0
            && out_alt == in_alt
            && self.out_rate_hz.load(Ordering::Relaxed) == self.in_rate_hz.load(Ordering::Relaxed)
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PacketClock {
    rate_hz: u32,
    bytes_per_audio_frame: usize,
    accumulator: u32,
}

impl PacketClock {
    pub const fn new() -> Self {
        Self {
            rate_hz: 0,
            bytes_per_audio_frame: 0,
            accumulator: 0,
        }
    }

    pub fn next_len(&mut self, rate_hz: u32, bytes_per_audio_frame: usize) -> usize {
        if self.rate_hz != rate_hz || self.bytes_per_audio_frame != bytes_per_audio_frame {
            self.rate_hz = rate_hz;
            self.bytes_per_audio_frame = bytes_per_audio_frame;
            self.accumulator = 0;
        }

        self.accumulator += rate_hz;
        let audio_frames = self.accumulator / 1_000;
        self.accumulator %= 1_000;
        audio_frames as usize * bytes_per_audio_frame
    }
}

impl Default for PacketClock {
    fn default() -> Self {
        Self::new()
    }
}

pub fn bytes_per_audio_frame(alt: u8) -> usize {
    match alt {
        SAMPLE_WIDTH_16_ALT => CHANNEL_COUNT as usize * 2,
        SAMPLE_WIDTH_24_ALT => CHANNEL_COUNT as usize * 3,
        _ => 0,
    }
}

pub fn closest_supported_rate(rate: u32) -> u32 {
    let mut closest = SUPPORTED_SAMPLE_RATES[0];
    let mut closest_diff = closest.abs_diff(rate);

    for candidate in SUPPORTED_SAMPLE_RATES.iter().copied().skip(1) {
        let diff = candidate.abs_diff(rate);
        if diff < closest_diff {
            closest = candidate;
            closest_diff = diff;
        }
    }

    closest
}

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::*;

    #[test]
    fn sample_width_alt_settings_map_to_audio_frame_sizes() {
        assert_eq!(bytes_per_audio_frame(0), 0);
        assert_eq!(bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT), 4);
        assert_eq!(bytes_per_audio_frame(SAMPLE_WIDTH_24_ALT), 6);
        assert_eq!(bytes_per_audio_frame(3), 0);
    }

    #[test]
    fn unsupported_sample_rates_round_to_nearest_supported_rate() {
        assert_eq!(closest_supported_rate(44_100), 44_100);
        assert_eq!(closest_supported_rate(48_000), 48_000);
        assert_eq!(closest_supported_rate(88_200), 88_200);
        assert_eq!(closest_supported_rate(96_000), 96_000);
        assert_eq!(closest_supported_rate(45_000), 44_100);
        assert_eq!(closest_supported_rate(50_000), 48_000);
        assert_eq!(closest_supported_rate(90_000), 88_200);
        assert_eq!(closest_supported_rate(95_000), 96_000);
    }

    #[test]
    fn packet_clock_emits_fractional_44k1_cadence() {
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..10 {
            let len = clock.next_len(44_100, bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT));
            total += len;
            if index == 9 {
                assert_eq!(len, 45 * 4);
            } else {
                assert_eq!(len, 44 * 4);
            }
        }

        assert_eq!(total, 441 * 4);
    }

    #[test]
    fn packet_clock_emits_fractional_88k2_cadence() {
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..5 {
            let len = clock.next_len(88_200, bytes_per_audio_frame(SAMPLE_WIDTH_24_ALT));
            total += len;
            if index == 4 {
                assert_eq!(len, 89 * 6);
            } else {
                assert_eq!(len, 88 * 6);
            }
        }

        assert_eq!(total, 441 * 6);
    }

    #[test]
    fn packet_clock_resets_fractional_state_when_format_changes() {
        let mut clock = PacketClock::new();

        assert_eq!(
            clock.next_len(44_100, bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT)),
            44 * 4
        );
        assert_eq!(
            clock.next_len(48_000, bytes_per_audio_frame(SAMPLE_WIDTH_16_ALT)),
            48 * 4
        );
        assert_eq!(
            clock.next_len(48_000, bytes_per_audio_frame(SAMPLE_WIDTH_24_ALT)),
            48 * 6
        );
    }

    #[test]
    fn loopback_only_matches_identical_active_formats() {
        let state = AudioState::new();
        assert!(!state.loopback_format_matches());

        state.set_alt(StreamDirection::Out, SAMPLE_WIDTH_16_ALT);
        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_16_ALT);
        assert!(state.loopback_format_matches());

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_24_ALT);
        assert!(!state.loopback_format_matches());

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_16_ALT);
        state.set_rate_hz(StreamDirection::In, 44_100);
        assert!(!state.loopback_format_matches());
    }
}
