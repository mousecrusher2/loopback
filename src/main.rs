#![no_main]
#![no_std]
#![cfg_attr(test, allow(dead_code, unused_imports))]

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

#[cfg(not(test))]
use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_rp::{bind_interrupts, peripherals, usb};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{
    Driver, Endpoint, EndpointError, EndpointIn, EndpointInfo, EndpointOut, EndpointType,
};
use embassy_usb::types::InterfaceNumber;
use embassy_usb::{Builder, Config, InterfaceAltBuilder};
#[cfg(not(test))]
use panic_halt as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
});

const USB_CLASS_AUDIO: u8 = 0x01;
const USB_SUBCLASS_AUDIO_CONTROL: u8 = 0x01;
const USB_SUBCLASS_AUDIO_STREAMING: u8 = 0x02;
const USB_AUDIO_PROTOCOL_UNDEFINED: u8 = 0x00;

const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;

const AC_HEADER: u8 = 0x01;
const AC_INPUT_TERMINAL: u8 = 0x02;
const AC_OUTPUT_TERMINAL: u8 = 0x03;
const AS_GENERAL: u8 = 0x01;
const FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const EP_GENERAL: u8 = 0x01;

const TERMINAL_USB_STREAMING: u16 = 0x0101;
const TERMINAL_MICROPHONE: u16 = 0x0201;
const TERMINAL_SPEAKER: u16 = 0x0301;
const FORMAT_PCM: u16 = 0x0001;
const CHANNEL_CONFIG_STEREO: u16 = 0x0003;

const OUT_USB_TERMINAL_ID: u8 = 1;
const OUT_SPEAKER_TERMINAL_ID: u8 = 2;
const IN_MIC_TERMINAL_ID: u8 = 3;
const IN_USB_TERMINAL_ID: u8 = 4;

const CHANNEL_COUNT: u8 = 2;
const SAMPLE_WIDTH_16_ALT: u8 = 1;
const SAMPLE_WIDTH_24_ALT: u8 = 2;
const DEFAULT_SAMPLE_RATE: u32 = 48_000;
const SUPPORTED_SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
const MAX_PACKET_SIZE_16: usize = 96_000 / 1_000 * CHANNEL_COUNT as usize * 2;
const MAX_PACKET_SIZE_24: usize = 96_000 / 1_000 * CHANNEL_COUNT as usize * 3;
const MAX_PACKET_SIZE: usize = MAX_PACKET_SIZE_24;
const PIPE_SIZE: usize = MAX_PACKET_SIZE * 8;

static AUDIO_STATE: AudioState = AudioState::new();
static AUDIO_PIPE: Pipe<CriticalSectionRawMutex, PIPE_SIZE> = Pipe::new();

struct AudioState {
    out_alt: AtomicU8,
    in_alt: AtomicU8,
    out_rate_hz: AtomicU32,
    in_rate_hz: AtomicU32,
}

impl AudioState {
    const fn new() -> Self {
        Self {
            out_alt: AtomicU8::new(0),
            in_alt: AtomicU8::new(0),
            out_rate_hz: AtomicU32::new(DEFAULT_SAMPLE_RATE),
            in_rate_hz: AtomicU32::new(DEFAULT_SAMPLE_RATE),
        }
    }

    fn in_bytes_per_audio_frame(&self) -> usize {
        bytes_per_audio_frame(self.in_alt.load(Ordering::Relaxed))
    }

    fn out_bytes_per_audio_frame(&self) -> usize {
        bytes_per_audio_frame(self.out_alt.load(Ordering::Relaxed))
    }

    fn loopback_format_matches(&self) -> bool {
        let out_alt = self.out_alt.load(Ordering::Relaxed);
        let in_alt = self.in_alt.load(Ordering::Relaxed);

        out_alt != 0
            && out_alt == in_alt
            && self.out_rate_hz.load(Ordering::Relaxed) == self.in_rate_hz.load(Ordering::Relaxed)
    }
}

struct AudioControlHandler {
    state: &'static AudioState,
    out_streaming_if: InterfaceNumber,
    in_streaming_if: InterfaceNumber,
    out_ep_addr: u8,
    in_ep_addr: u8,
}

impl AudioControlHandler {
    fn new(
        state: &'static AudioState,
        out_streaming_if: InterfaceNumber,
        in_streaming_if: InterfaceNumber,
        out_ep_addr: u8,
        in_ep_addr: u8,
    ) -> Self {
        Self {
            state,
            out_streaming_if,
            in_streaming_if,
            out_ep_addr,
            in_ep_addr,
        }
    }

    fn rate_for_endpoint(&self, ep_addr: u8) -> Option<&AtomicU32> {
        if ep_addr == self.out_ep_addr {
            Some(&self.state.out_rate_hz)
        } else if ep_addr == self.in_ep_addr {
            Some(&self.state.in_rate_hz)
        } else {
            None
        }
    }
}

impl embassy_usb::Handler for AudioControlHandler {
    fn reset(&mut self) {
        self.state.out_alt.store(0, Ordering::Relaxed);
        self.state.in_alt.store(0, Ordering::Relaxed);
        self.state
            .out_rate_hz
            .store(DEFAULT_SAMPLE_RATE, Ordering::Relaxed);
        self.state
            .in_rate_hz
            .store(DEFAULT_SAMPLE_RATE, Ordering::Relaxed);
    }

    fn set_alternate_setting(&mut self, iface: InterfaceNumber, alternate_setting: u8) {
        if iface == self.out_streaming_if {
            self.state
                .out_alt
                .store(alternate_setting, Ordering::Relaxed);
            AUDIO_PIPE.clear();
        } else if iface == self.in_streaming_if {
            self.state
                .in_alt
                .store(alternate_setting, Ordering::Relaxed);
            AUDIO_PIPE.clear();
        }
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        let ep_addr = (req.index & 0xff) as u8;
        let control_selector = (req.value >> 8) as u8;

        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Endpoint
            || req.request != 0x01
            || control_selector != 0x01
        {
            return None;
        }

        let Some(rate) = self.rate_for_endpoint(ep_addr) else {
            return None;
        };

        if data.len() != 3 {
            return Some(OutResponse::Rejected);
        }

        let requested = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
        rate.store(closest_supported_rate(requested), Ordering::Relaxed);
        AUDIO_PIPE.clear();
        Some(OutResponse::Accepted)
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        let ep_addr = (req.index & 0xff) as u8;
        let control_selector = (req.value >> 8) as u8;

        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Endpoint
            || control_selector != 0x01
        {
            return None;
        }

        let Some(rate) = self.rate_for_endpoint(ep_addr) else {
            return None;
        };

        let value = match req.request {
            0x81 => rate.load(Ordering::Relaxed),
            0x82 => 44_100,
            0x83 => 96_000,
            0x84 => 1,
            _ => return Some(InResponse::Rejected),
        };

        if buf.len() < 3 {
            return Some(InResponse::Rejected);
        }

        buf[0] = value as u8;
        buf[1] = (value >> 8) as u8;
        buf[2] = (value >> 16) as u8;
        Some(InResponse::Accepted(&buf[..3]))
    }
}

struct AudioEndpoints<'d, D: Driver<'d>> {
    out_ep: D::EndpointOut,
    in_ep: D::EndpointIn,
    out_streaming_if: InterfaceNumber,
    in_streaming_if: InterfaceNumber,
    out_ep_addr: u8,
    in_ep_addr: u8,
}

fn build_audio_function<'d, D: Driver<'d>>(builder: &mut Builder<'d, D>) -> AudioEndpoints<'d, D> {
    let mut function = builder.function(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_CONTROL,
        USB_AUDIO_PROTOCOL_UNDEFINED,
    );

    let mut control_if = function.interface();
    let control_if_number = control_if.interface_number();
    let out_streaming_if_number = u8::from(control_if_number) + 1;
    let in_streaming_if_number = u8::from(control_if_number) + 2;

    {
        let mut alt = control_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_CONTROL,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
        write_audio_control_descriptors(&mut alt, out_streaming_if_number, in_streaming_if_number);
    }
    drop(control_if);

    let mut out_if = function.interface();
    let out_streaming_if = out_if.interface_number();
    {
        let _alt0 = out_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_STREAMING,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
    }

    let out_ep;
    let out_info;
    {
        let mut alt16 = out_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_STREAMING,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
        write_streaming_descriptors(&mut alt16, OUT_USB_TERMINAL_ID, 2, 16);
        out_ep =
            alt16.alloc_endpoint_out(EndpointType::Isochronous, None, MAX_PACKET_SIZE as u16, 1);
        out_info = *out_ep.info();
        write_audio_data_endpoint(
            &mut alt16,
            &out_info,
            MAX_PACKET_SIZE_16 as u16,
            SynchronizationType::Synchronous,
        );
        write_class_specific_endpoint(&mut alt16);
    }
    {
        let mut alt24 = out_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_STREAMING,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
        write_streaming_descriptors(&mut alt24, OUT_USB_TERMINAL_ID, 3, 24);
        write_audio_data_endpoint(
            &mut alt24,
            &out_info,
            MAX_PACKET_SIZE_24 as u16,
            SynchronizationType::Synchronous,
        );
        write_class_specific_endpoint(&mut alt24);
    }
    drop(out_if);

    let mut in_if = function.interface();
    let in_streaming_if = in_if.interface_number();
    {
        let _alt0 = in_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_STREAMING,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
    }

    let in_ep;
    let in_info;
    {
        let mut alt16 = in_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_STREAMING,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
        write_streaming_descriptors(&mut alt16, IN_USB_TERMINAL_ID, 2, 16);
        in_ep = alt16.alloc_endpoint_in(EndpointType::Isochronous, None, MAX_PACKET_SIZE as u16, 1);
        in_info = *in_ep.info();
        write_audio_data_endpoint(
            &mut alt16,
            &in_info,
            MAX_PACKET_SIZE_16 as u16,
            SynchronizationType::Synchronous,
        );
        write_class_specific_endpoint(&mut alt16);
    }
    {
        let mut alt24 = in_if.alt_setting(
            USB_CLASS_AUDIO,
            USB_SUBCLASS_AUDIO_STREAMING,
            USB_AUDIO_PROTOCOL_UNDEFINED,
            None,
        );
        write_streaming_descriptors(&mut alt24, IN_USB_TERMINAL_ID, 3, 24);
        write_audio_data_endpoint(
            &mut alt24,
            &in_info,
            MAX_PACKET_SIZE_24 as u16,
            SynchronizationType::Synchronous,
        );
        write_class_specific_endpoint(&mut alt24);
    }

    AudioEndpoints {
        out_ep,
        in_ep,
        out_streaming_if,
        in_streaming_if,
        out_ep_addr: u8::from(out_info.addr),
        in_ep_addr: u8::from(in_info.addr),
    }
}

fn write_audio_control_descriptors<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    out_streaming_if: u8,
    in_streaming_if: u8,
) {
    const AC_TOTAL_LENGTH: u16 = 52;

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_HEADER,
            0x00,
            0x01,
            AC_TOTAL_LENGTH as u8,
            (AC_TOTAL_LENGTH >> 8) as u8,
            0x02,
            out_streaming_if,
            in_streaming_if,
        ],
    );

    write_input_terminal(
        alt,
        OUT_USB_TERMINAL_ID,
        TERMINAL_USB_STREAMING,
        CHANNEL_COUNT,
        CHANNEL_CONFIG_STEREO,
    );
    write_output_terminal(
        alt,
        OUT_SPEAKER_TERMINAL_ID,
        TERMINAL_SPEAKER,
        OUT_USB_TERMINAL_ID,
    );
    write_input_terminal(
        alt,
        IN_MIC_TERMINAL_ID,
        TERMINAL_MICROPHONE,
        CHANNEL_COUNT,
        CHANNEL_CONFIG_STEREO,
    );
    write_output_terminal(
        alt,
        IN_USB_TERMINAL_ID,
        TERMINAL_USB_STREAMING,
        IN_MIC_TERMINAL_ID,
    );
}

fn write_input_terminal<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_id: u8,
    terminal_type: u16,
    channel_count: u8,
    channel_config: u16,
) {
    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_INPUT_TERMINAL,
            terminal_id,
            terminal_type as u8,
            (terminal_type >> 8) as u8,
            0x00,
            channel_count,
            channel_config as u8,
            (channel_config >> 8) as u8,
            0x00,
            0x00,
        ],
    );
}

fn write_output_terminal<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_id: u8,
    terminal_type: u16,
    source_id: u8,
) {
    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_OUTPUT_TERMINAL,
            terminal_id,
            terminal_type as u8,
            (terminal_type >> 8) as u8,
            0x00,
            source_id,
            0x00,
        ],
    );
}

fn write_streaming_descriptors<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_link: u8,
    subframe_size: u8,
    bit_resolution: u8,
) {
    alt.descriptor(
        CS_INTERFACE,
        &[
            AS_GENERAL,
            terminal_link,
            0x01,
            FORMAT_PCM as u8,
            (FORMAT_PCM >> 8) as u8,
        ],
    );

    alt.descriptor(
        CS_INTERFACE,
        &[
            FORMAT_TYPE,
            FORMAT_TYPE_I,
            CHANNEL_COUNT,
            subframe_size,
            bit_resolution,
            0x04,
            0x44,
            0xac,
            0x00,
            0x80,
            0xbb,
            0x00,
            0x88,
            0x58,
            0x01,
            0x00,
            0x77,
            0x01,
        ],
    );
}

fn write_class_specific_endpoint<'d, D: Driver<'d>>(alt: &mut InterfaceAltBuilder<'_, 'd, D>) {
    alt.descriptor(CS_ENDPOINT, &[EP_GENERAL, 0x01, 0x00, 0x00, 0x00]);
}

fn write_audio_data_endpoint<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    endpoint: &EndpointInfo,
    max_packet_size: u16,
    synchronization_type: SynchronizationType,
) {
    let mut endpoint = *endpoint;
    endpoint.max_packet_size = max_packet_size;
    alt.endpoint_descriptor(
        &endpoint,
        synchronization_type,
        UsageType::DataEndpoint,
        &[0, 0],
    );
}

async fn out_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointOut,
    state: &'static AudioState,
    pipe: &'static Pipe<CriticalSectionRawMutex, PIPE_SIZE>,
) {
    let mut packet = [0u8; MAX_PACKET_SIZE];

    loop {
        ep.wait_enabled().await;

        loop {
            match ep.read(&mut packet).await {
                Ok(len) => {
                    if len == 0 {
                        continue;
                    }
                    let bytes_per_audio_frame = state.out_bytes_per_audio_frame();
                    if !state.loopback_format_matches()
                        || bytes_per_audio_frame == 0
                        || len % bytes_per_audio_frame != 0
                    {
                        pipe.clear();
                    } else if pipe.try_write(&packet[..len]).is_err() {
                        pipe.clear();
                        let _ = pipe.try_write(&packet[..len]);
                    }
                }
                Err(EndpointError::Disabled) => break,
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}

async fn in_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointIn,
    state: &'static AudioState,
    pipe: &'static Pipe<CriticalSectionRawMutex, PIPE_SIZE>,
) {
    let mut packet = [0u8; MAX_PACKET_SIZE];
    let mut clock = PacketClock::new();

    loop {
        ep.wait_enabled().await;

        loop {
            let bytes_per_audio_frame = state.in_bytes_per_audio_frame();
            if bytes_per_audio_frame == 0 {
                embassy_futures::yield_now().await;
                continue;
            }

            let packet_len = clock.next_len(
                state.in_rate_hz.load(Ordering::Relaxed),
                bytes_per_audio_frame,
            );
            packet[..packet_len].fill(0);

            if state.loopback_format_matches() {
                let mut offset = 0;
                while offset < packet_len {
                    match pipe.try_read(&mut packet[offset..packet_len]) {
                        Ok(0) => break,
                        Ok(read) => offset += read,
                        Err(_) => break,
                    }
                }
            } else {
                pipe.clear();
            }

            match ep.write(&packet[..packet_len]).await {
                Ok(()) => {}
                Err(EndpointError::Disabled) => break,
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}

struct PacketClock {
    rate_hz: u32,
    bytes_per_audio_frame: usize,
    accumulator: u32,
}

impl PacketClock {
    const fn new() -> Self {
        Self {
            rate_hz: 0,
            bytes_per_audio_frame: 0,
            accumulator: 0,
        }
    }

    fn next_len(&mut self, rate_hz: u32, bytes_per_audio_frame: usize) -> usize {
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

fn bytes_per_audio_frame(alt: u8) -> usize {
    match alt {
        SAMPLE_WIDTH_16_ALT => CHANNEL_COUNT as usize * 2,
        SAMPLE_WIDTH_24_ALT => CHANNEL_COUNT as usize * 3,
        _ => 0,
    }
}

fn closest_supported_rate(rate: u32) -> u32 {
    let mut closest = 44_100u32;
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

        state.out_alt.store(SAMPLE_WIDTH_16_ALT, Ordering::Relaxed);
        state.in_alt.store(SAMPLE_WIDTH_16_ALT, Ordering::Relaxed);
        assert!(state.loopback_format_matches());

        state.in_alt.store(SAMPLE_WIDTH_24_ALT, Ordering::Relaxed);
        assert!(!state.loopback_format_matches());

        state.in_alt.store(SAMPLE_WIDTH_16_ALT, Ordering::Relaxed);
        state.in_rate_hz.store(44_100, Ordering::Relaxed);
        assert!(!state.loopback_format_matches());
    }
}

#[cfg(not(test))]
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = usb::Driver::new(p.USB, Irqs);

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 512]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 32]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 32]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HANDLER: StaticCell<AudioControlHandler> = StaticCell::new();

    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 512]);
    let bos_descriptor = BOS_DESCRIPTOR.init([0; 32]);
    let msos_descriptor = MSOS_DESCRIPTOR.init([0; 32]);
    let control_buf = CONTROL_BUF.init([0; 64]);

    let mut config = Config::new(0xcafe, 0x4001);
    config.manufacturer = Some("Embassy");
    config.product = Some("Pico 2 UAC1 Loopback");
    config.serial_number = Some("pico2-loopback-0001");
    config.max_packet_size_0 = 64;
    config.max_power = 100;

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buf,
    );

    let endpoints = build_audio_function(&mut builder);
    let handler = HANDLER.init(AudioControlHandler::new(
        &AUDIO_STATE,
        endpoints.out_streaming_if,
        endpoints.in_streaming_if,
        endpoints.out_ep_addr,
        endpoints.in_ep_addr,
    ));
    builder.handler(handler);

    let mut usb = builder.build();

    join3(
        usb.run(),
        out_task::<usb::Driver<'static, peripherals::USB>>(
            endpoints.out_ep,
            &AUDIO_STATE,
            &AUDIO_PIPE,
        ),
        in_task::<usb::Driver<'static, peripherals::USB>>(
            endpoints.in_ep,
            &AUDIO_STATE,
            &AUDIO_PIPE,
        ),
    )
    .await;
}
