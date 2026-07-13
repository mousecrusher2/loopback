use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{Driver, Endpoint, EndpointAddress, EndpointInfo, EndpointType};
use embassy_usb::types::InterfaceNumber;
use embassy_usb::{Builder, InterfaceAltBuilder, InterfaceBuilder};

use crate::spec::{self, PcmFormat};

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

const FORMAT_PCM: u16 = 0x0001;

#[derive(Clone, Copy)]
#[repr(transparent)]
struct AudioEntityId(u8);

#[derive(Clone, Copy)]
#[repr(u16)]
enum TerminalType {
    UsbStreaming = 0x0101,
    LineConnector = 0x0603,
}

impl TerminalType {
    const fn to_le_bytes(self) -> [u8; 2] {
        (self as u16).to_le_bytes()
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
struct ChannelConfig(u16);

impl ChannelConfig {
    const STEREO: Self = Self(0x0003);

    const fn to_le_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }
}

const PLAYBACK_USB_TERMINAL_ID: AudioEntityId = AudioEntityId(1);
const PLAYBACK_LINE_TERMINAL_ID: AudioEntityId = AudioEntityId(2);
const CAPTURE_LINE_TERMINAL_ID: AudioEntityId = AudioEntityId(3);
const CAPTURE_USB_TERMINAL_ID: AudioEntityId = AudioEntityId(4);

#[derive(Clone, Copy)]
pub(crate) struct AudioControlMap {
    pub(crate) playback_interface: InterfaceNumber,
    pub(crate) capture_interface: InterfaceNumber,
    pub(crate) playback_endpoint_addresses: [EndpointAddress; spec::FORMAT_COUNT],
    pub(crate) capture_endpoint_addresses: [EndpointAddress; spec::FORMAT_COUNT],
}

pub(crate) struct AudioEndpoints<'d, D: Driver<'d>> {
    pub(crate) playback: [D::EndpointOut; spec::FORMAT_COUNT],
    pub(crate) capture: [D::EndpointIn; spec::FORMAT_COUNT],
    playback_interface: InterfaceNumber,
    capture_interface: InterfaceNumber,
}

impl<'d, D: Driver<'d>> AudioEndpoints<'d, D> {
    pub(crate) fn control_map(&self) -> AudioControlMap {
        AudioControlMap {
            playback_interface: self.playback_interface,
            capture_interface: self.capture_interface,
            playback_endpoint_addresses: self
                .playback
                .each_ref()
                .map(|endpoint| endpoint.info().addr),
            capture_endpoint_addresses: self
                .capture
                .each_ref()
                .map(|endpoint| endpoint.info().addr),
        }
    }
}

pub(crate) fn build_audio_function<'d, D: Driver<'d>>(
    builder: &mut Builder<'d, D>,
) -> AudioEndpoints<'d, D> {
    let mut function = builder.function(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_CONTROL,
        USB_AUDIO_PROTOCOL_UNDEFINED,
    );

    let mut control = function.interface();
    let control_number = control.interface_number();
    // FunctionBuilder guarantees that interface numbers are consecutive.
    let playback_interface = InterfaceNumber(control_number.0 + 1);
    let capture_interface = InterfaceNumber(control_number.0 + 2);
    let mut control_alt = control.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_CONTROL,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    write_audio_control(&mut control_alt, playback_interface, capture_interface);

    let mut playback = function.interface();
    let playback_interface = playback.interface_number();
    write_zero_bandwidth_alt(&mut playback);
    let playback =
        core::array::from_fn(|slot| write_playback_alt(&mut playback, &spec::PCM_FORMATS[slot]));

    let mut capture = function.interface();
    let capture_interface = capture.interface_number();
    write_zero_bandwidth_alt(&mut capture);
    let capture =
        core::array::from_fn(|slot| write_capture_alt(&mut capture, &spec::PCM_FORMATS[slot]));

    AudioEndpoints {
        playback,
        capture,
        playback_interface,
        capture_interface,
    }
}

fn write_zero_bandwidth_alt<'d, D: Driver<'d>>(interface: &mut InterfaceBuilder<'_, 'd, D>) {
    let _ = interface.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
}

fn write_playback_alt<'d, D: Driver<'d>>(
    interface: &mut InterfaceBuilder<'_, 'd, D>,
    format: &PcmFormat,
) -> D::EndpointOut {
    let mut alt = stream_alt(interface);
    write_streaming_descriptors(&mut alt, PLAYBACK_USB_TERMINAL_ID, format);
    let max_packet_size = format.max_packet_size();
    let endpoint = alt.alloc_endpoint_out(EndpointType::Isochronous, None, max_packet_size, 1);
    let endpoint_info = *endpoint.info();
    write_audio_endpoint(
        &mut alt,
        &endpoint_info,
        max_packet_size,
        // Host-paced OUT is intentional; see docs/uac1-design.md.
        SynchronizationType::Adaptive,
    );
    write_sampling_frequency_control(&mut alt);
    endpoint
}

fn write_capture_alt<'d, D: Driver<'d>>(
    interface: &mut InterfaceBuilder<'_, 'd, D>,
    format: &PcmFormat,
) -> D::EndpointIn {
    let mut alt = stream_alt(interface);
    write_streaming_descriptors(&mut alt, CAPTURE_USB_TERMINAL_ID, format);
    let max_packet_size = format.max_packet_size();
    let endpoint = alt.alloc_endpoint_in(EndpointType::Isochronous, None, max_packet_size, 1);
    let endpoint_info = *endpoint.info();
    write_audio_endpoint(
        &mut alt,
        &endpoint_info,
        max_packet_size,
        // Loopback IN follows queued OUT payload lengths rather than promising a
        // fixed synchronous sample count per SOF.
        SynchronizationType::Asynchronous,
    );
    write_sampling_frequency_control(&mut alt);
    endpoint
}

fn stream_alt<'a, 'b, 'd, D: Driver<'d>>(
    interface: &'a mut InterfaceBuilder<'b, 'd, D>,
) -> InterfaceAltBuilder<'a, 'd, D> {
    interface.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    )
}

fn write_audio_control<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    playback_interface: InterfaceNumber,
    capture_interface: InterfaceNumber,
) {
    const AC_TOTAL_LENGTH: u16 = 52;
    let [total_lo, total_hi] = AC_TOTAL_LENGTH.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_HEADER,
            0x00,
            0x01,
            total_lo,
            total_hi,
            0x02,
            playback_interface.0,
            capture_interface.0,
        ],
    );

    write_input_terminal(
        alt,
        PLAYBACK_USB_TERMINAL_ID,
        TerminalType::UsbStreaming,
        spec::CHANNELS,
        ChannelConfig::STEREO,
    );
    write_output_terminal(
        alt,
        PLAYBACK_LINE_TERMINAL_ID,
        TerminalType::LineConnector,
        PLAYBACK_USB_TERMINAL_ID,
    );
    write_input_terminal(
        alt,
        CAPTURE_LINE_TERMINAL_ID,
        TerminalType::LineConnector,
        spec::CHANNELS,
        ChannelConfig::STEREO,
    );
    write_output_terminal(
        alt,
        CAPTURE_USB_TERMINAL_ID,
        TerminalType::UsbStreaming,
        CAPTURE_LINE_TERMINAL_ID,
    );
}

fn write_input_terminal<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_id: AudioEntityId,
    terminal_type: TerminalType,
    channels: u8,
    channel_config: ChannelConfig,
) {
    let [type_lo, type_hi] = terminal_type.to_le_bytes();
    let [config_lo, config_hi] = channel_config.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_INPUT_TERMINAL,
            terminal_id.0,
            type_lo,
            type_hi,
            0x00,
            channels,
            config_lo,
            config_hi,
            0x00,
            0x00,
        ],
    );
}

fn write_output_terminal<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_id: AudioEntityId,
    terminal_type: TerminalType,
    source_id: AudioEntityId,
) {
    let [type_lo, type_hi] = terminal_type.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_OUTPUT_TERMINAL,
            terminal_id.0,
            type_lo,
            type_hi,
            0x00,
            source_id.0,
            0x00,
        ],
    );
}

fn write_streaming_descriptors<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_link: AudioEntityId,
    format: &PcmFormat,
) {
    let [pcm_lo, pcm_hi] = FORMAT_PCM.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[AS_GENERAL, terminal_link.0, 0x01, pcm_lo, pcm_hi],
    );

    let mut descriptor = [0_u8; spec::FORMAT_DESCRIPTOR_CAPACITY];
    descriptor[0] = FORMAT_TYPE;
    descriptor[1] = FORMAT_TYPE_I;
    descriptor[2] = spec::CHANNELS;
    descriptor[3] = format.sample.byte_width();
    descriptor[4] = format.sample.bit_resolution();
    descriptor[5] = u8::try_from(format.rates.len()).expect("sample-rate count fits u8");

    let mut offset = 6;
    for rate in format.rates {
        let [b0, b1, b2, 0] = rate.hz().to_le_bytes() else {
            unreachable!("UAC1 sample rates fit in 24 bits");
        };
        descriptor[offset..offset + 3].copy_from_slice(&[b0, b1, b2]);
        offset += 3;
    }

    alt.descriptor(CS_INTERFACE, &descriptor[..offset]);
}

fn write_sampling_frequency_control<'d, D: Driver<'d>>(alt: &mut InterfaceAltBuilder<'_, 'd, D>) {
    alt.descriptor(CS_ENDPOINT, &[EP_GENERAL, 0x01, 0x00, 0x00, 0x00]);
}

fn write_audio_endpoint<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    endpoint: &EndpointInfo,
    max_packet_size: u16,
    synchronization_type: SynchronizationType,
) {
    let mut descriptor_endpoint = *endpoint;
    descriptor_endpoint.max_packet_size = max_packet_size;
    alt.endpoint_descriptor(
        &descriptor_endpoint,
        synchronization_type,
        UsageType::DataEndpoint,
        &[0x00, 0x00],
    );
}
