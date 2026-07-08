pub(crate) const USB_VENDOR_ID: u16 = 0xcafe;
pub(crate) const USB_PRODUCT_ID: u16 = 0x4001;
pub(crate) const USB_MANUFACTURER: &str = "Embassy";
pub(crate) const USB_PRODUCT: &str = "Pico 2 UAC1 Loopback";
pub(crate) const USB_SERIAL: &str = "pico2-loopback-0005";
pub(crate) const USB_MAX_PACKET_SIZE_0: u8 = 64;
pub(crate) const USB_MAX_POWER_MA: u16 = 100;

pub(crate) const CHANNELS: u8 = 2;
pub(crate) const PACKET_QUEUE_CAPACITY: usize = 16;
pub(crate) const DEFAULT_RATE: SampleRate = SampleRate::R48000;

const RATES_UP_TO_96K: [SampleRate; 4] = [
    SampleRate::R44100,
    SampleRate::R48000,
    SampleRate::R88200,
    SampleRate::R96000,
];
const RATES_UP_TO_48K: [SampleRate; 2] = [SampleRate::R44100, SampleRate::R48000];

pub(crate) const PCM_FORMATS: [PcmFormat; 3] = [
    PcmFormat {
        alternate_setting: 1,
        sample: SampleWidth::Bits16,
        rates: &RATES_UP_TO_96K,
        max_packet_size: 388,
    },
    PcmFormat {
        alternate_setting: 2,
        sample: SampleWidth::Bits24,
        rates: &RATES_UP_TO_96K,
        max_packet_size: 582,
    },
    PcmFormat {
        alternate_setting: 3,
        sample: SampleWidth::Bits32,
        rates: &RATES_UP_TO_48K,
        max_packet_size: 392,
    },
];

pub(crate) const MAX_AUDIO_PACKET_BYTES: usize = 582;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum StreamDirection {
    Playback,
    Capture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SampleWidth {
    Bits16,
    Bits24,
    Bits32,
}

impl SampleWidth {
    pub(crate) const fn byte_width(self) -> u8 {
        match self {
            Self::Bits16 => 2,
            Self::Bits24 => 3,
            Self::Bits32 => 4,
        }
    }

    pub(crate) const fn bit_resolution(self) -> u8 {
        match self {
            Self::Bits16 => 16,
            Self::Bits24 => 24,
            Self::Bits32 => 32,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PcmFormat {
    pub(crate) alternate_setting: u8,
    pub(crate) sample: SampleWidth,
    pub(crate) rates: &'static [SampleRate],
    pub(crate) max_packet_size: u16,
}

impl PcmFormat {
    pub(crate) const fn audio_frame_bytes(self) -> usize {
        CHANNELS as usize * self.sample.byte_width() as usize
    }

    pub(crate) fn supports(self, rate: SampleRate) -> bool {
        self.rates.contains(&rate)
    }

    pub(crate) fn max_rate(self) -> SampleRate {
        self.rates[self.rates.len() - 1]
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SampleRate {
    R44100 = 0,
    #[default]
    R48000 = 1,
    R88200 = 2,
    R96000 = 3,
}

impl SampleRate {
    pub(crate) const fn hz(self) -> u32 {
        match self {
            Self::R44100 => 44_100,
            Self::R48000 => 48_000,
            Self::R88200 => 88_200,
            Self::R96000 => 96_000,
        }
    }

    pub(crate) const fn from_hz(hz: u32) -> Option<Self> {
        match hz {
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
            _ => DEFAULT_RATE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamSelection {
    alternate_setting: u8,
    rate: SampleRate,
}

impl StreamSelection {
    pub(crate) const fn inactive() -> Self {
        Self {
            alternate_setting: 0,
            rate: DEFAULT_RATE,
        }
    }

    pub(crate) const fn new(alternate_setting: u8, rate: SampleRate) -> Self {
        Self {
            alternate_setting,
            rate,
        }
    }

    pub(crate) const fn alternate_setting(self) -> u8 {
        self.alternate_setting
    }

    pub(crate) const fn rate(self) -> SampleRate {
        self.rate
    }

    pub(crate) fn format(self) -> Option<&'static PcmFormat> {
        format_by_alternate_setting(self.alternate_setting)
    }

    const fn encode(self) -> u8 {
        (self.rate.code() << 4) | (self.alternate_setting & 0x0f)
    }

    const fn decode(bits: u8) -> Self {
        Self::new(bits & 0x0f, SampleRate::from_code(bits >> 4))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DuplexSelection {
    pub(crate) playback: StreamSelection,
    pub(crate) capture: StreamSelection,
}

impl DuplexSelection {
    pub(crate) const fn inactive() -> Self {
        Self {
            playback: StreamSelection::inactive(),
            capture: StreamSelection::inactive(),
        }
    }

    pub(crate) fn loopback_enabled(self) -> bool {
        self.playback.alternate_setting != 0
            && self.playback.alternate_setting == self.capture.alternate_setting
            && self.playback.rate == self.capture.rate
    }

    pub(crate) const fn stream(self, direction: StreamDirection) -> StreamSelection {
        match direction {
            StreamDirection::Playback => self.playback,
            StreamDirection::Capture => self.capture,
        }
    }

    pub(crate) const fn with_stream(
        mut self,
        direction: StreamDirection,
        stream: StreamSelection,
    ) -> Self {
        match direction {
            StreamDirection::Playback => self.playback = stream,
            StreamDirection::Capture => self.capture = stream,
        }
        self
    }

    pub(crate) const fn encode(self) -> u32 {
        (self.playback.encode() as u32) | ((self.capture.encode() as u32) << 8)
    }

    pub(crate) const fn decode(bits: u32) -> Self {
        Self {
            playback: StreamSelection::decode((bits & 0xff) as u8),
            capture: StreamSelection::decode(((bits >> 8) & 0xff) as u8),
        }
    }
}

pub(crate) fn format_by_alternate_setting(alternate_setting: u8) -> Option<&'static PcmFormat> {
    PCM_FORMATS
        .iter()
        .find(|format| format.alternate_setting == alternate_setting)
}

pub(crate) fn format_by_endpoint_slot(slot: usize) -> Option<&'static PcmFormat> {
    PCM_FORMATS.get(slot)
}

pub(crate) fn rate_or_default_for_format(rate: SampleRate, format: &PcmFormat) -> SampleRate {
    if format.supports(rate) {
        rate
    } else {
        DEFAULT_RATE
    }
}
