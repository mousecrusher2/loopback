use core::sync::atomic::{AtomicU32, Ordering};

pub const CHANNEL_COUNT: u8 = 2;
pub const SAMPLE_WIDTH_16_ALT: u8 = 1;
pub const SAMPLE_WIDTH_24_ALT: u8 = 2;
pub const SAMPLE_WIDTH_32_ALT: u8 = 3;
pub const DEFAULT_SAMPLE_RATE: SampleRate = SampleRate::R48000;
pub const SUPPORTED_SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
pub const SUPPORTED_SAMPLE_RATES_32: [u32; 2] = [44_100, 48_000];
const SUPPORTED_SAMPLE_RATE_CODES: [SampleRate; 4] = [
    SampleRate::R44100,
    SampleRate::R48000,
    SampleRate::R88200,
    SampleRate::R96000,
];
const SUPPORTED_SAMPLE_RATE_CODES_32: [SampleRate; 2] = [SampleRate::R44100, SampleRate::R48000];
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

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleRate {
    R44100 = 44_100,
    R48000 = 48_000,
    R88200 = 88_200,
    R96000 = 96_000,
}

impl SampleRate {
    pub const fn hz(self) -> u32 {
        self as u32
    }

    const fn code(self) -> u8 {
        match self {
            Self::R44100 => 0,
            Self::R48000 => 1,
            Self::R88200 => 2,
            Self::R96000 => 3,
        }
    }

    const fn from_code(code: u8) -> Self {
        match code & 0x0f {
            0 => Self::R44100,
            1 => Self::R48000,
            2 => Self::R88200,
            3 => Self::R96000,
            _ => DEFAULT_SAMPLE_RATE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamFormat {
    pub alt: u8,
    pub rate: SampleRate,
}

impl StreamFormat {
    pub const fn new(alt: u8, rate: SampleRate) -> Self {
        Self { alt, rate }
    }

    pub fn bytes_per_audio_frame(self) -> usize {
        bytes_per_audio_frame(self.alt)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioFormats {
    pub out: StreamFormat,
    pub in_: StreamFormat,
}

impl AudioFormats {
    pub const fn new(out: StreamFormat, in_: StreamFormat) -> Self {
        Self { out, in_ }
    }

    pub fn loopback_format_matches(self) -> bool {
        self.out.alt != 0 && self.out.alt == self.in_.alt && self.out.rate == self.in_.rate
    }
}

pub struct AudioState {
    formats: AtomicU32,
}

#[derive(Clone, Copy)]
enum StreamFormatUpdate {
    Alt(u8),
    Rate(SampleRate),
}

impl AudioState {
    pub const fn new() -> Self {
        Self {
            formats: AtomicU32::new(encode_audio_formats(default_audio_formats())),
        }
    }

    pub fn reset(&self) {
        self.formats.store(
            encode_audio_formats(default_audio_formats()),
            Ordering::Relaxed,
        );
    }

    pub fn set_alt(&self, direction: StreamDirection, alternate_setting: u8) -> StreamFormat {
        self.update_format(direction, StreamFormatUpdate::Alt(alternate_setting))
    }

    pub fn set_rate(&self, direction: StreamDirection, rate: SampleRate) -> StreamFormat {
        self.update_format(direction, StreamFormatUpdate::Rate(rate))
    }

    pub fn formats(&self) -> AudioFormats {
        decode_audio_formats(self.formats.load(Ordering::Relaxed))
    }

    fn update_format(
        &self,
        direction: StreamDirection,
        update: StreamFormatUpdate,
    ) -> StreamFormat {
        let previous_bits =
            self.formats
                .update(Ordering::Relaxed, Ordering::Relaxed, |current_bits| {
                    let (formats, _) = apply_stream_format_update(
                        decode_audio_formats(current_bits),
                        direction,
                        update,
                    );
                    encode_audio_formats(formats)
                });

        let (_, stored_format) =
            apply_stream_format_update(decode_audio_formats(previous_bits), direction, update);
        stored_format
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

const fn default_audio_formats() -> AudioFormats {
    AudioFormats::new(
        StreamFormat::new(0, DEFAULT_SAMPLE_RATE),
        StreamFormat::new(0, DEFAULT_SAMPLE_RATE),
    )
}

fn normalize_stream_format(format: StreamFormat) -> StreamFormat {
    let alt = match format.alt {
        0 | SAMPLE_WIDTH_16_ALT | SAMPLE_WIDTH_24_ALT | SAMPLE_WIDTH_32_ALT => format.alt,
        _ => 0,
    };
    let rate = if alt == SAMPLE_WIDTH_32_ALT {
        closest_rate_in(format.rate.hz(), &SUPPORTED_SAMPLE_RATE_CODES_32)
    } else {
        format.rate
    };
    StreamFormat::new(alt, rate)
}

fn apply_stream_format_update(
    mut formats: AudioFormats,
    direction: StreamDirection,
    update: StreamFormatUpdate,
) -> (AudioFormats, StreamFormat) {
    let current = match direction {
        StreamDirection::Out => formats.out,
        StreamDirection::In => formats.in_,
    };
    let next_format = normalize_stream_format(match update {
        StreamFormatUpdate::Alt(alt) => StreamFormat::new(alt, current.rate),
        StreamFormatUpdate::Rate(rate) => StreamFormat::new(current.alt, rate),
    });

    match direction {
        StreamDirection::Out => formats.out = next_format,
        StreamDirection::In => formats.in_ = next_format,
    }

    (formats, next_format)
}

const fn encode_audio_formats(formats: AudioFormats) -> u32 {
    encode_stream_format(formats.out) | (encode_stream_format(formats.in_) << 8)
}

const fn encode_stream_format(format: StreamFormat) -> u32 {
    ((format.rate.code() as u32) << 4) | ((format.alt as u32) & 0x0f)
}

const fn decode_audio_formats(bits: u32) -> AudioFormats {
    AudioFormats::new(
        decode_stream_format(bits as u8),
        decode_stream_format((bits >> 8) as u8),
    )
}

const fn decode_stream_format(bits: u8) -> StreamFormat {
    StreamFormat::new(bits & 0x0f, SampleRate::from_code(bits >> 4))
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

pub fn closest_supported_rate(rate: u32) -> SampleRate {
    closest_rate_in(rate, &SUPPORTED_SAMPLE_RATE_CODES)
}

pub fn closest_supported_rate_for_alt(alt: u8, rate: u32) -> SampleRate {
    if alt == SAMPLE_WIDTH_32_ALT {
        closest_rate_in(rate, &SUPPORTED_SAMPLE_RATE_CODES_32)
    } else {
        closest_supported_rate(rate)
    }
}

fn closest_rate_in(rate: u32, rates: &[SampleRate]) -> SampleRate {
    let mut closest = rates[0];
    let mut closest_diff = closest.hz().abs_diff(rate);

    for candidate in rates.iter().copied().skip(1) {
        let diff = candidate.hz().abs_diff(rate);
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
        assert_eq!(closest_supported_rate(44_100), SampleRate::R44100);
        assert_eq!(closest_supported_rate(48_000), SampleRate::R48000);
        assert_eq!(closest_supported_rate(88_200), SampleRate::R88200);
        assert_eq!(closest_supported_rate(96_000), SampleRate::R96000);
        assert_eq!(closest_supported_rate(45_000), SampleRate::R44100);
        assert_eq!(closest_supported_rate(50_000), SampleRate::R48000);
        assert_eq!(closest_supported_rate(90_000), SampleRate::R88200);
        assert_eq!(closest_supported_rate(95_000), SampleRate::R96000);
    }

    #[test]
    fn unsupported_32bit_sample_rates_round_to_44k1_or_48k() {
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 44_100),
            SampleRate::R44100
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 48_000),
            SampleRate::R48000
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 45_000),
            SampleRate::R44100
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 50_000),
            SampleRate::R48000
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 88_200),
            SampleRate::R48000
        );
        assert_eq!(
            closest_supported_rate_for_alt(SAMPLE_WIDTH_32_ALT, 96_000),
            SampleRate::R48000
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
        assert!(!state.formats().loopback_format_matches());

        state.set_alt(StreamDirection::Out, SAMPLE_WIDTH_16_ALT);
        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_16_ALT);
        assert!(state.formats().loopback_format_matches());

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_24_ALT);
        assert!(!state.formats().loopback_format_matches());

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_16_ALT);
        state.set_rate(StreamDirection::In, SampleRate::R44100);
        assert!(!state.formats().loopback_format_matches());

        state.set_alt(StreamDirection::Out, SAMPLE_WIDTH_32_ALT);
        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_32_ALT);
        state.set_rate(StreamDirection::Out, SampleRate::R48000);
        state.set_rate(StreamDirection::In, SampleRate::R48000);
        assert!(state.formats().loopback_format_matches());
    }

    #[test]
    fn stream_format_snapshot_keeps_32bit_rate_within_packet_budget() {
        let state = AudioState::new();

        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_24_ALT);
        assert_eq!(
            state.set_rate(StreamDirection::In, SampleRate::R96000).rate,
            SampleRate::R96000
        );

        let stored = state.set_alt(StreamDirection::In, SAMPLE_WIDTH_32_ALT);
        let format = state.formats().in_;

        assert_eq!(stored.rate, SampleRate::R48000);
        assert_eq!(format.alt, SAMPLE_WIDTH_32_ALT);
        assert_eq!(format.rate, SampleRate::R48000);
        assert!(
            PacketClock::new().next_len(format.rate.hz(), format.bytes_per_audio_frame())
                <= MAX_PACKET_SIZE
        );
    }

    #[test]
    fn audio_formats_snapshot_contains_out_and_in_together() {
        let state = AudioState::new();

        state.set_alt(StreamDirection::Out, SAMPLE_WIDTH_24_ALT);
        state.set_rate(StreamDirection::Out, SampleRate::R88200);
        state.set_alt(StreamDirection::In, SAMPLE_WIDTH_24_ALT);
        state.set_rate(StreamDirection::In, SampleRate::R88200);

        let formats = state.formats();

        assert_eq!(
            formats.out,
            StreamFormat::new(SAMPLE_WIDTH_24_ALT, SampleRate::R88200)
        );
        assert_eq!(
            formats.in_,
            StreamFormat::new(SAMPLE_WIDTH_24_ALT, SampleRate::R88200)
        );
        assert!(formats.loopback_format_matches());
    }
}
