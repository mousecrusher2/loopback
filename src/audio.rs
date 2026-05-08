use core::sync::atomic::{AtomicU32, Ordering};

pub const CHANNEL_COUNT: u8 = 2;
pub const SAMPLE_WIDTH_16_ALT: u8 = 1;
pub const SAMPLE_WIDTH_24_ALT: u8 = 2;
pub const SAMPLE_WIDTH_32_ALT: u8 = 3;
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
pub const SUPPORTED_SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
pub const SUPPORTED_SAMPLE_RATES_32: [u32; 2] = [44_100, 48_000];
pub const MAX_PACKET_SIZE_16: usize = 97 * CHANNEL_COUNT as usize * 2;
pub const MAX_PACKET_SIZE_24: usize = 97 * CHANNEL_COUNT as usize * 3;
pub const MAX_PACKET_SIZE_32: usize = 49 * CHANNEL_COUNT as usize * 4;
pub const MAX_PACKET_SIZE: usize = MAX_PACKET_SIZE_24;
pub const PIPE_SIZE: usize = MAX_PACKET_SIZE * 16;
pub const PACKET_LEN_QUEUE_SIZE: usize = 64;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum StreamDirection {
    Out,
    In,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct StreamFormat {
    pub alt: u8,
    pub rate_hz: u32,
}

impl StreamFormat {
    pub const fn new(alt: u8, rate_hz: u32) -> Self {
        Self { alt, rate_hz }
    }

    pub fn bytes_per_audio_frame(self) -> usize {
        bytes_per_audio_frame(self.alt)
    }
}

pub struct AudioState {
    out: AtomicU32,
    in_: AtomicU32,
}

impl AudioState {
    pub const fn new() -> Self {
        Self {
            out: AtomicU32::new(encode_stream_format(StreamFormat::new(
                0,
                DEFAULT_SAMPLE_RATE,
            ))),
            in_: AtomicU32::new(encode_stream_format(StreamFormat::new(
                0,
                DEFAULT_SAMPLE_RATE,
            ))),
        }
    }

    pub fn reset(&self) {
        self.store_format(
            StreamDirection::Out,
            StreamFormat::new(0, DEFAULT_SAMPLE_RATE),
        );
        self.store_format(
            StreamDirection::In,
            StreamFormat::new(0, DEFAULT_SAMPLE_RATE),
        );
    }

    pub fn set_alt(&self, direction: StreamDirection, alternate_setting: u8) -> u32 {
        let current = self.format(direction);
        let rate_hz = closest_supported_rate_for_alt(alternate_setting, current.rate_hz);
        self.store_format(direction, StreamFormat::new(alternate_setting, rate_hz));
        rate_hz
    }

    pub fn set_rate_hz(&self, direction: StreamDirection, rate_hz: u32) -> u32 {
        let current = self.format(direction);
        let rate_hz = closest_supported_rate_for_alt(current.alt, rate_hz);
        self.store_format(direction, StreamFormat::new(current.alt, rate_hz));
        rate_hz
    }

    pub fn rate_hz(&self, direction: StreamDirection) -> u32 {
        self.format(direction).rate_hz
    }

    pub fn format(&self, direction: StreamDirection) -> StreamFormat {
        let bits = match direction {
            StreamDirection::Out => self.out.load(Ordering::Relaxed),
            StreamDirection::In => self.in_.load(Ordering::Relaxed),
        };
        decode_stream_format(bits)
    }

    fn store_format(&self, direction: StreamDirection, format: StreamFormat) {
        match direction {
            StreamDirection::Out => self
                .out
                .store(encode_stream_format(format), Ordering::Relaxed),
            StreamDirection::In => self
                .in_
                .store(encode_stream_format(format), Ordering::Relaxed),
        }
    }

    pub fn in_bytes_per_audio_frame(&self) -> usize {
        self.format(StreamDirection::In).bytes_per_audio_frame()
    }

    pub fn out_bytes_per_audio_frame(&self) -> usize {
        self.format(StreamDirection::Out).bytes_per_audio_frame()
    }

    pub fn loopback_format_matches(&self) -> bool {
        let out = self.format(StreamDirection::Out);
        let in_ = self.format(StreamDirection::In);

        out.alt != 0 && out.alt == in_.alt && out.rate_hz == in_.rate_hz
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

const fn encode_stream_format(format: StreamFormat) -> u32 {
    (format.rate_hz << 8) | format.alt as u32
}

const fn decode_stream_format(bits: u32) -> StreamFormat {
    StreamFormat::new((bits & 0xff) as u8, bits >> 8)
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
        SAMPLE_WIDTH_32_ALT => CHANNEL_COUNT as usize * 4,
        _ => 0,
    }
}

pub fn closest_supported_rate(rate: u32) -> u32 {
    closest_rate_in(rate, &SUPPORTED_SAMPLE_RATES)
}

pub fn closest_supported_rate_for_alt(alt: u8, rate: u32) -> u32 {
    if alt == SAMPLE_WIDTH_32_ALT {
        closest_rate_in(rate, &SUPPORTED_SAMPLE_RATES_32)
    } else {
        closest_supported_rate(rate)
    }
}

fn closest_rate_in(rate: u32, rates: &[u32]) -> u32 {
    let mut closest = rates[0];
    let mut closest_diff = closest.abs_diff(rate);

    for candidate in rates.iter().copied().skip(1) {
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
        assert_eq!(bytes_per_audio_frame(SAMPLE_WIDTH_32_ALT), 8);
        assert_eq!(bytes_per_audio_frame(4), 0);
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
    fn unsupported_32bit_sample_rates_round_to_44k1_or_48k() {
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 44_100),
            44_100
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 48_000),
            48_000
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 45_000),
            44_100
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 50_000),
            48_000
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 88_200),
            48_000
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 96_000),
            48_000
        );
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
    fn packet_clock_emits_32bit_fractional_44k1_cadence() {
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..10 {
            let len = clock.next_len(44_100, bytes_per_audio_frame(SAMPLE_WIDTH_32_ALT));
            total += len;
            if index == 9 {
                assert_eq!(len, 45 * 8);
            } else {
                assert_eq!(len, 44 * 8);
            }
        }

        assert_eq!(total, 441 * 8);
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

        state.set_alt(StreamDirection::Out, SAMPLE_WIDTH_32_ALT);
        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_32_ALT);
        state.set_rate_hz(StreamDirection::Out, 48_000);
        state.set_rate_hz(StreamDirection::In, 48_000);
        assert!(state.loopback_format_matches());
    }

    #[test]
    fn stream_format_snapshot_keeps_32bit_rate_within_packet_budget() {
        let state = AudioState::new();

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_24_ALT);
        assert_eq!(state.set_rate_hz(StreamDirection::In, 96_000), 96_000);

        let rate = state.set_alt(StreamDirection::In, SAMPLE_WIDTH_32_ALT);
        let format = state.format(StreamDirection::In);

        assert_eq!(rate, 48_000);
        assert_eq!(format.alt, SAMPLE_WIDTH_32_ALT);
        assert_eq!(format.rate_hz, 48_000);
        assert!(
            PacketClock::new().next_len(format.rate_hz, format.bytes_per_audio_frame())
                <= MAX_PACKET_SIZE
        );
    }
}
