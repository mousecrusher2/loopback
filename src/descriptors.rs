use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{Driver, Endpoint, EndpointInfo, EndpointType};
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

const TERMINAL_USB_STREAMING: u16 = 0x0101;
const TERMINAL_MICROPHONE: u16 = 0x0201;
const TERMINAL_SPEAKER: u16 = 0x0301;
const FORMAT_PCM: u16 = 0x0001;
const CHANNEL_CONFIG_STEREO: u16 = 0x0003;

const PLAYBACK_USB_TERMINAL_ID: u8 = 1;
const PLAYBACK_SPEAKER_TERMINAL_ID: u8 = 2;
const CAPTURE_MIC_TERMINAL_ID: u8 = 3;
const CAPTURE_USB_TERMINAL_ID: u8 = 4;

#[derive(Clone, Copy)]
pub(crate) struct AudioRouting {
    pub(crate) playback_interface: InterfaceNumber,
    pub(crate) capture_interface: InterfaceNumber,
    pub(crate) playback_endpoints: [u8; 3],
    pub(crate) capture_endpoints: [u8; 3],
}

pub(crate) struct AudioEndpoints<'d, D: Driver<'d>> {
    pub(crate) playback_16: D::EndpointOut,
    pub(crate) playback_24: D::EndpointOut,
    pub(crate) playback_32: D::EndpointOut,
    pub(crate) capture_16: D::EndpointIn,
    pub(crate) capture_24: D::EndpointIn,
    pub(crate) capture_32: D::EndpointIn,
    pub(crate) routing: AudioRouting,
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
    let playback_interface = u8::from(control_number) + 1;
    let capture_interface = u8::from(control_number) + 2;
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
    let (playback_16, playback_16_addr) = write_playback_alt(&mut playback, &spec::PCM_FORMATS[0]);
    let (playback_24, playback_24_addr) = write_playback_alt(&mut playback, &spec::PCM_FORMATS[1]);
    let (playback_32, playback_32_addr) = write_playback_alt(&mut playback, &spec::PCM_FORMATS[2]);

    let mut capture = function.interface();
    let capture_interface = capture.interface_number();
    write_zero_bandwidth_alt(&mut capture);
    let (capture_16, capture_16_addr) = write_capture_alt(&mut capture, &spec::PCM_FORMATS[0]);
    let (capture_24, capture_24_addr) = write_capture_alt(&mut capture, &spec::PCM_FORMATS[1]);
    let (capture_32, capture_32_addr) = write_capture_alt(&mut capture, &spec::PCM_FORMATS[2]);

    AudioEndpoints {
        playback_16,
        playback_24,
        playback_32,
        capture_16,
        capture_24,
        capture_32,
        routing: AudioRouting {
            playback_interface,
            capture_interface,
            playback_endpoints: [playback_16_addr, playback_24_addr, playback_32_addr],
            capture_endpoints: [capture_16_addr, capture_24_addr, capture_32_addr],
        },
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
) -> (D::EndpointOut, u8) {
    let mut alt = stream_alt(interface);
    write_streaming_descriptors(&mut alt, PLAYBACK_USB_TERMINAL_ID, format);
    let endpoint =
        alt.alloc_endpoint_out(EndpointType::Isochronous, None, format.max_packet_size, 1);
    let endpoint_info = *endpoint.info();
    write_audio_endpoint(
        &mut alt,
        &endpoint_info,
        format.max_packet_size,
        SynchronizationType::Adaptive,
    );
    write_sampling_frequency_control(&mut alt);
    (endpoint, u8::from(endpoint_info.addr))
}

fn write_capture_alt<'d, D: Driver<'d>>(
    interface: &mut InterfaceBuilder<'_, 'd, D>,
    format: &PcmFormat,
) -> (D::EndpointIn, u8) {
    let mut alt = stream_alt(interface);
    write_streaming_descriptors(&mut alt, CAPTURE_USB_TERMINAL_ID, format);
    let endpoint =
        alt.alloc_endpoint_in(EndpointType::Isochronous, None, format.max_packet_size, 1);
    let endpoint_info = *endpoint.info();
    write_audio_endpoint(
        &mut alt,
        &endpoint_info,
        format.max_packet_size,
        SynchronizationType::Asynchronous,
    );
    write_sampling_frequency_control(&mut alt);
    (endpoint, u8::from(endpoint_info.addr))
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
    playback_interface: u8,
    capture_interface: u8,
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
            playback_interface,
            capture_interface,
        ],
    );

    write_input_terminal(
        alt,
        PLAYBACK_USB_TERMINAL_ID,
        TERMINAL_USB_STREAMING,
        spec::CHANNELS,
        CHANNEL_CONFIG_STEREO,
    );
    write_output_terminal(
        alt,
        PLAYBACK_SPEAKER_TERMINAL_ID,
        TERMINAL_SPEAKER,
        PLAYBACK_USB_TERMINAL_ID,
    );
    write_input_terminal(
        alt,
        CAPTURE_MIC_TERMINAL_ID,
        TERMINAL_MICROPHONE,
        spec::CHANNELS,
        CHANNEL_CONFIG_STEREO,
    );
    write_output_terminal(
        alt,
        CAPTURE_USB_TERMINAL_ID,
        TERMINAL_USB_STREAMING,
        CAPTURE_MIC_TERMINAL_ID,
    );
}

fn write_input_terminal<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_id: u8,
    terminal_type: u16,
    channels: u8,
    channel_config: u16,
) {
    let [type_lo, type_hi] = terminal_type.to_le_bytes();
    let [config_lo, config_hi] = channel_config.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_INPUT_TERMINAL,
            terminal_id,
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
    terminal_id: u8,
    terminal_type: u16,
    source_id: u8,
) {
    let [type_lo, type_hi] = terminal_type.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_OUTPUT_TERMINAL,
            terminal_id,
            type_lo,
            type_hi,
            0x00,
            source_id,
            0x00,
        ],
    );
}

fn write_streaming_descriptors<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_link: u8,
    format: &PcmFormat,
) {
    let [pcm_lo, pcm_hi] = FORMAT_PCM.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[AS_GENERAL, terminal_link, 0x01, pcm_lo, pcm_hi],
    );

    let mut descriptor = [0_u8; 18];
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
