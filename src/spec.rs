pub(crate) const USB_VENDOR_ID: u16 = 0xcafe;
pub(crate) const USB_PRODUCT_ID: u16 = 0x4001;
pub(crate) const USB_MANUFACTURER: &str = "Embassy";
pub(crate) const USB_PRODUCT: &str = "Pico 2 UAC1 Loopback";
pub(crate) const USB_MAX_PACKET_SIZE_0: u8 = 64;
pub(crate) const USB_MAX_POWER_MA: u16 = 100;

pub(crate) const CHANNELS: u8 = 2;
pub(crate) const PACKET_QUEUE_CAPACITY: usize = 8;
pub(crate) const DEFAULT_RATE: SampleRate = SampleRate::R48000;

const RATES_UP_TO_96K: [SampleRate; 4] = [
    SampleRate::R44100,
    SampleRate::R48000,
    SampleRate::R88200,
    SampleRate::R96000,
];
const RATES_UP_TO_48K: [SampleRate; 2] = [SampleRate::R44100, SampleRate::R48000];

// Keep formats and rates in one table. Alternate numbers, endpoints, task pools,
// MPS values, and descriptor capacities are all derived from its order/content.
pub(crate) const PCM_FORMATS: &[PcmFormat] = &[
    PcmFormat {
        sample: SampleWidth::Bits16,
        rates: &RATES_UP_TO_96K,
    },
    PcmFormat {
        sample: SampleWidth::Bits24,
        rates: &RATES_UP_TO_96K,
    },
    PcmFormat {
        sample: SampleWidth::Bits32,
        rates: &RATES_UP_TO_48K,
    },
];

pub(crate) const FORMAT_COUNT: usize = PCM_FORMATS.len();
pub(crate) const FORMAT_RATE_COUNT: usize = format_rate_count();
pub(crate) const MAX_AUDIO_PACKET_BYTES: usize = max_audio_packet_bytes();
pub(crate) const MAX_RATES_PER_FORMAT: usize = max_rates_per_format();
pub(crate) const FORMAT_DESCRIPTOR_CAPACITY: usize = 6 + 3 * MAX_RATES_PER_FORMAT;
pub(crate) const CONFIG_DESCRIPTOR_CAPACITY: usize = config_descriptor_capacity();

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
    pub(crate) sample: SampleWidth,
    pub(crate) rates: &'static [SampleRate],
}

impl PcmFormat {
    pub(crate) const fn audio_frame_bytes(self) -> usize {
        CHANNELS as usize * self.sample.byte_width() as usize
    }

    pub(crate) fn supports(self, rate: SampleRate) -> bool {
        self.rates.contains(&rate)
    }

    pub(crate) fn default_rate(self) -> SampleRate {
        if self.supports(DEFAULT_RATE) {
            DEFAULT_RATE
        } else {
            self.min_rate()
        }
    }

    pub(crate) const fn max_rate(self) -> SampleRate {
        let mut maximum = self.rates[0];
        let mut index = 1;
        while index < self.rates.len() {
            let rate = self.rates[index];
            if rate.hz() > maximum.hz() {
                maximum = rate;
            }
            index += 1;
        }
        maximum
    }

    pub(crate) const fn min_rate(self) -> SampleRate {
        let mut minimum = self.rates[0];
        let mut index = 1;
        while index < self.rates.len() {
            let rate = self.rates[index];
            if rate.hz() < minimum.hz() {
                minimum = rate;
            }
            index += 1;
        }
        minimum
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(crate) const fn max_packet_size(self) -> u16 {
        let frames = self.max_rate().hz() / 1_000 + 1;
        let bytes = frames as usize * self.audio_frame_bytes();
        bytes as u16
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
}

pub(crate) fn format_by_endpoint_slot(slot: usize) -> Option<&'static PcmFormat> {
    PCM_FORMATS.get(slot)
}

pub(crate) const fn format_slot_by_alternate_setting(alternate_setting: u8) -> Option<usize> {
    if alternate_setting == 0 || alternate_setting as usize > FORMAT_COUNT {
        None
    } else {
        Some(alternate_setting as usize - 1)
    }
}

pub(crate) fn audio_queue_index(format_slot: usize, rate: SampleRate) -> Option<usize> {
    let format = format_by_endpoint_slot(format_slot)?;
    let mut offset = 0;
    let mut slot = 0;
    while slot < format_slot {
        offset += PCM_FORMATS[slot].rates.len();
        slot += 1;
    }

    let mut rate_slot = 0;
    while rate_slot < format.rates.len() {
        if format.rates[rate_slot] == rate {
            return Some(offset + rate_slot);
        }
        rate_slot += 1;
    }

    None
}

const fn max_audio_packet_bytes() -> usize {
    let mut maximum = 0;
    let mut slot = 0;
    while slot < FORMAT_COUNT {
        let packet_size = PCM_FORMATS[slot].max_packet_size() as usize;
        if packet_size > maximum {
            maximum = packet_size;
        }
        slot += 1;
    }
    maximum
}

const fn format_rate_count() -> usize {
    let mut total = 0;
    let mut slot = 0;
    while slot < FORMAT_COUNT {
        total += PCM_FORMATS[slot].rates.len();
        slot += 1;
    }
    total
}

const fn max_rates_per_format() -> usize {
    let mut maximum = 0;
    let mut slot = 0;
    while slot < FORMAT_COUNT {
        let rate_count = PCM_FORMATS[slot].rates.len();
        if rate_count > maximum {
            maximum = rate_count;
        }
        slot += 1;
    }
    maximum
}

const fn config_descriptor_capacity() -> usize {
    // Configuration, IAD, AudioControl, and both zero-bandwidth interfaces.
    let mut total = 96;
    let mut slot = 0;
    while slot < FORMAT_COUNT {
        // Each direction has one interface, AS general, format, standard endpoint,
        // and class-specific endpoint descriptor for this format.
        total += 2 * (40 + 3 * PCM_FORMATS[slot].rates.len());
        slot += 1;
    }
    total
}
