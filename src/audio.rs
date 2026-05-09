use core::sync::atomic::{AtomicU32, Ordering};

pub const CHANNEL_COUNT: u8 = 2;
pub const DEFAULT_SAMPLE_RATE: SampleRate = SampleRate::R48000;
pub const SUPPORTED_SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
pub const SUPPORTED_SAMPLE_RATES_32: [u32; 2] = [44_100, 48_000];
const DEFAULT_STREAM_FORMAT: StreamFormat = StreamFormat {
    alternate_setting: AudioStreamingAlternateSetting::Inactive,
    rate: DEFAULT_SAMPLE_RATE,
};
const DEFAULT_AUDIO_FORMATS: AudioFormats = AudioFormats {
    out: DEFAULT_STREAM_FORMAT,
    in_: DEFAULT_STREAM_FORMAT,
};
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

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AudioStreamingAlternateSetting {
    #[default]
    Inactive = 0,
    Pcm16 = 1,
    Pcm24 = 2,
    Pcm32 = 3,
}

impl AudioStreamingAlternateSetting {
    pub const fn from_number(number: u8) -> Option<Self> {
        match number {
            0 => Some(Self::Inactive),
            1 => Some(Self::Pcm16),
            2 => Some(Self::Pcm24),
            3 => Some(Self::Pcm32),
            _ => None,
        }
    }

    pub const fn number(self) -> u8 {
        self as u8
    }

    pub const fn bytes_per_audio_frame(self) -> usize {
        match self {
            Self::Inactive => 0,
            Self::Pcm16 => CHANNEL_COUNT as usize * 2,
            Self::Pcm24 => CHANNEL_COUNT as usize * 3,
            Self::Pcm32 => CHANNEL_COUNT as usize * 4,
        }
    }

    pub const fn supports_sample_rate(self, rate: SampleRate) -> bool {
        match self {
            Self::Pcm32 => matches!(rate, SampleRate::R44100 | SampleRate::R48000),
            _ => true,
        }
    }

    pub const fn sample_rate_or_default(self, rate: SampleRate) -> SampleRate {
        if self.supports_sample_rate(rate) {
            rate
        } else {
            DEFAULT_SAMPLE_RATE
        }
    }

    const fn from_code(code: u8) -> Self {
        match code & 0x0f {
            0 => Self::Inactive,
            1 => Self::Pcm16,
            2 => Self::Pcm24,
            3 => Self::Pcm32,
            _ => Self::Inactive,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SampleRate {
    R44100 = 0,
    #[default]
    R48000 = 1,
    R88200 = 2,
    R96000 = 3,
}

impl SampleRate {
    pub const fn hz(self) -> u32 {
        match self {
            Self::R44100 => 44_100,
            Self::R48000 => 48_000,
            Self::R88200 => 88_200,
            Self::R96000 => 96_000,
        }
    }

    pub const fn from_hz(rate_hz: u32) -> Option<Self> {
        match rate_hz {
            44_100 => Some(Self::R44100),
            48_000 => Some(Self::R48000),
            88_200 => Some(Self::R88200),
            96_000 => Some(Self::R96000),
            _ => None,
        }
    }

    const fn code(self) -> u8 {
        self as u8
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamFormat {
    pub alternate_setting: AudioStreamingAlternateSetting,
    pub rate: SampleRate,
}

impl StreamFormat {
    pub const fn new(alternate_setting: AudioStreamingAlternateSetting, rate: SampleRate) -> Self {
        Self {
            alternate_setting,
            rate,
        }
    }

    pub fn bytes_per_audio_frame(self) -> usize {
        self.alternate_setting.bytes_per_audio_frame()
    }

    const fn encode(self) -> u32 {
        ((self.rate.code() as u32) << 4) | ((self.alternate_setting.number() as u32) & 0x0f)
    }

    const fn decode(bits: u8) -> Self {
        Self::new(
            AudioStreamingAlternateSetting::from_code(bits),
            SampleRate::from_code(bits >> 4),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AudioFormats {
    pub out: StreamFormat,
    pub in_: StreamFormat,
}

impl AudioFormats {
    pub const fn new(out: StreamFormat, in_: StreamFormat) -> Self {
        Self { out, in_ }
    }

    pub fn loopback_format_matches(self) -> bool {
        self.out.alternate_setting != AudioStreamingAlternateSetting::Inactive
            && self.out.alternate_setting == self.in_.alternate_setting
            && self.out.rate == self.in_.rate
    }

    const fn encode(self) -> u32 {
        self.out.encode() | (self.in_.encode() << 8)
    }

    const fn decode(bits: u32) -> Self {
        Self::new(
            StreamFormat::decode(bits as u8),
            StreamFormat::decode((bits >> 8) as u8),
        )
    }
}

pub struct AudioState {
    formats: AtomicU32,
}

#[derive(Clone, Copy)]
enum StreamFormatUpdate {
    AlternateSetting(AudioStreamingAlternateSetting),
    Rate(SampleRate),
}

impl AudioState {
    pub const fn new() -> Self {
        Self {
            formats: AtomicU32::new(DEFAULT_AUDIO_FORMATS.encode()),
        }
    }

    pub fn reset(&self) {
        self.formats
            .store(AudioFormats::default().encode(), Ordering::Relaxed);
    }

    pub fn set_alternate_setting(
        &self,
        direction: StreamDirection,
        alternate_setting: AudioStreamingAlternateSetting,
    ) -> StreamFormat {
        match self.update_format(
            direction,
            StreamFormatUpdate::AlternateSetting(alternate_setting),
        ) {
            Some(format) => format,
            None => unreachable!("alternate setting updates are always valid"),
        }
    }

    pub fn set_rate(&self, direction: StreamDirection, rate: SampleRate) -> Option<StreamFormat> {
        self.update_format(direction, StreamFormatUpdate::Rate(rate))
    }

    pub fn formats(&self) -> AudioFormats {
        AudioFormats::decode(self.formats.load(Ordering::Relaxed))
    }

    fn update_format(
        &self,
        direction: StreamDirection,
        update: StreamFormatUpdate,
    ) -> Option<StreamFormat> {
        let previous_bits = self
            .formats
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current_bits| {
                let (formats, _) = apply_stream_format_update(
                    AudioFormats::decode(current_bits),
                    direction,
                    update,
                )?;
                Some(formats.encode())
            })
            .ok()?;

        let (_, stored_format) =
            apply_stream_format_update(AudioFormats::decode(previous_bits), direction, update)?;
        Some(stored_format)
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_stream_format_update(
    mut formats: AudioFormats,
    direction: StreamDirection,
    update: StreamFormatUpdate,
) -> Option<(AudioFormats, StreamFormat)> {
    let current = match direction {
        StreamDirection::Out => formats.out,
        StreamDirection::In => formats.in_,
    };
    let next_format = match update {
        StreamFormatUpdate::AlternateSetting(alternate_setting) => StreamFormat::new(
            alternate_setting,
            alternate_setting.sample_rate_or_default(current.rate),
        ),
        StreamFormatUpdate::Rate(rate) => {
            if !current.alternate_setting.supports_sample_rate(rate) {
                return None;
            }
            StreamFormat::new(current.alternate_setting, rate)
        }
    };

    match direction {
        StreamDirection::Out => formats.out = next_format,
        StreamDirection::In => formats.in_ = next_format,
    }

    Some((formats, next_format))
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

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::*;

    #[test]
    fn audio_streaming_alternate_settings_map_to_audio_frame_sizes() {
        assert_eq!(
            AudioStreamingAlternateSetting::Inactive.bytes_per_audio_frame(),
            0
        );
        assert_eq!(
            AudioStreamingAlternateSetting::Pcm16.bytes_per_audio_frame(),
            4
        );
        assert_eq!(
            AudioStreamingAlternateSetting::Pcm24.bytes_per_audio_frame(),
            6
        );
        assert_eq!(
            AudioStreamingAlternateSetting::Pcm32.bytes_per_audio_frame(),
            8
        );
    }

    #[test]
    fn default_formats_are_inactive_48k_streams() {
        assert_eq!(
            AudioFormats::default(),
            AudioFormats::new(
                StreamFormat::new(AudioStreamingAlternateSetting::Inactive, SampleRate::R48000),
                StreamFormat::new(AudioStreamingAlternateSetting::Inactive, SampleRate::R48000)
            )
        );
    }

    #[test]
    fn sample_rate_from_hz_accepts_only_advertised_rates() {
        assert_eq!(SampleRate::from_hz(44_100), Some(SampleRate::R44100));
        assert_eq!(SampleRate::from_hz(48_000), Some(SampleRate::R48000));
        assert_eq!(SampleRate::from_hz(88_200), Some(SampleRate::R88200));
        assert_eq!(SampleRate::from_hz(96_000), Some(SampleRate::R96000));
        assert_eq!(SampleRate::from_hz(45_000), None);
        assert_eq!(SampleRate::from_hz(50_000), None);
        assert_eq!(SampleRate::from_hz(90_000), None);
        assert_eq!(SampleRate::from_hz(95_000), None);
    }

    #[test]
    fn sample_rate_support_depends_on_alt_setting() {
        assert!(AudioStreamingAlternateSetting::Pcm32.supports_sample_rate(SampleRate::R44100));
        assert!(AudioStreamingAlternateSetting::Pcm32.supports_sample_rate(SampleRate::R48000));
        assert!(!AudioStreamingAlternateSetting::Pcm32.supports_sample_rate(SampleRate::R88200));
        assert!(!AudioStreamingAlternateSetting::Pcm32.supports_sample_rate(SampleRate::R96000));
        assert!(AudioStreamingAlternateSetting::Pcm24.supports_sample_rate(SampleRate::R96000));
        assert_eq!(
            AudioStreamingAlternateSetting::Pcm32.sample_rate_or_default(SampleRate::R96000),
            DEFAULT_SAMPLE_RATE
        );
    }

    #[test]
    fn packet_clock_emits_fractional_44k1_cadence() {
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..10 {
            let len = clock.next_len(
                44_100,
                AudioStreamingAlternateSetting::Pcm16.bytes_per_audio_frame(),
            );
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
            let len = clock.next_len(
                88_200,
                AudioStreamingAlternateSetting::Pcm24.bytes_per_audio_frame(),
            );
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
            let len = clock.next_len(
                44_100,
                AudioStreamingAlternateSetting::Pcm32.bytes_per_audio_frame(),
            );
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
            clock.next_len(
                44_100,
                AudioStreamingAlternateSetting::Pcm16.bytes_per_audio_frame()
            ),
            44 * 4
        );
        assert_eq!(
            clock.next_len(
                48_000,
                AudioStreamingAlternateSetting::Pcm16.bytes_per_audio_frame()
            ),
            48 * 4
        );
        assert_eq!(
            clock.next_len(
                48_000,
                AudioStreamingAlternateSetting::Pcm24.bytes_per_audio_frame()
            ),
            48 * 6
        );
    }

    #[test]
    fn loopback_only_matches_identical_active_formats() {
        let state = AudioState::new();
        assert!(!state.formats().loopback_format_matches());

        state.set_alternate_setting(StreamDirection::Out, AudioStreamingAlternateSetting::Pcm16);
        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm16);
        assert!(state.formats().loopback_format_matches());

        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm24);
        assert!(!state.formats().loopback_format_matches());

        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm16);
        assert_eq!(
            state.set_rate(StreamDirection::In, SampleRate::R44100),
            Some(StreamFormat::new(
                AudioStreamingAlternateSetting::Pcm16,
                SampleRate::R44100
            ))
        );
        assert!(!state.formats().loopback_format_matches());

        state.set_alternate_setting(StreamDirection::Out, AudioStreamingAlternateSetting::Pcm32);
        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm32);
        assert_eq!(
            state.set_rate(StreamDirection::Out, SampleRate::R48000),
            Some(StreamFormat::new(
                AudioStreamingAlternateSetting::Pcm32,
                SampleRate::R48000
            ))
        );
        assert_eq!(
            state.set_rate(StreamDirection::In, SampleRate::R48000),
            Some(StreamFormat::new(
                AudioStreamingAlternateSetting::Pcm32,
                SampleRate::R48000
            ))
        );
        assert!(state.formats().loopback_format_matches());
    }

    #[test]
    fn stream_format_snapshot_keeps_32bit_rate_within_packet_budget() {
        let state = AudioState::new();

        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm24);
        assert_eq!(
            state.set_rate(StreamDirection::In, SampleRate::R96000),
            Some(StreamFormat::new(
                AudioStreamingAlternateSetting::Pcm24,
                SampleRate::R96000
            ))
        );

        let stored =
            state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm32);
        let format = state.formats().in_;

        assert_eq!(stored.rate, SampleRate::R48000);
        assert_eq!(
            format.alternate_setting,
            AudioStreamingAlternateSetting::Pcm32
        );
        assert_eq!(format.rate, SampleRate::R48000);
        assert!(
            PacketClock::new().next_len(format.rate.hz(), format.bytes_per_audio_frame())
                <= MAX_PACKET_SIZE
        );
    }

    #[test]
    fn unsupported_rate_update_is_rejected_for_32bit_streams() {
        let state = AudioState::new();

        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm32);

        assert_eq!(
            state.set_rate(StreamDirection::In, SampleRate::R96000),
            None
        );
        assert_eq!(
            state.formats().in_,
            StreamFormat::new(AudioStreamingAlternateSetting::Pcm32, SampleRate::R48000)
        );
    }

    #[test]
    fn audio_formats_snapshot_contains_out_and_in_together() {
        let state = AudioState::new();

        state.set_alternate_setting(StreamDirection::Out, AudioStreamingAlternateSetting::Pcm24);
        assert_eq!(
            state.set_rate(StreamDirection::Out, SampleRate::R88200),
            Some(StreamFormat::new(
                AudioStreamingAlternateSetting::Pcm24,
                SampleRate::R88200
            ))
        );
        state.set_alternate_setting(StreamDirection::In, AudioStreamingAlternateSetting::Pcm24);
        assert_eq!(
            state.set_rate(StreamDirection::In, SampleRate::R88200),
            Some(StreamFormat::new(
                AudioStreamingAlternateSetting::Pcm24,
                SampleRate::R88200
            ))
        );

        let formats = state.formats();

        assert_eq!(
            formats.out,
            StreamFormat::new(AudioStreamingAlternateSetting::Pcm24, SampleRate::R88200)
        );
        assert_eq!(
            formats.in_,
            StreamFormat::new(AudioStreamingAlternateSetting::Pcm24, SampleRate::R88200)
        );
        assert!(formats.loopback_format_matches());
    }
}
