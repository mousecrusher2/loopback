use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{Driver, Endpoint, EndpointInfo, EndpointType};
use embassy_usb::types::InterfaceNumber;
use embassy_usb::{Builder, InterfaceAltBuilder};

use crate::audio::{
    BitDepth, CHANNEL_COUNT, bit_resolution, subframe_size, supported_sample_rates,
};

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

pub(crate) struct AudioEndpoints<'d, D: Driver<'d>> {
    pub(crate) out_ep16: D::EndpointOut,
    pub(crate) out_ep24: D::EndpointOut,
    pub(crate) out_ep32: D::EndpointOut,
    pub(crate) in_ep16: D::EndpointIn,
    pub(crate) in_ep24: D::EndpointIn,
    pub(crate) in_ep32: D::EndpointIn,
    pub(crate) out_streaming_if: InterfaceNumber,
    pub(crate) in_streaming_if: InterfaceNumber,
    pub(crate) out_ep_addrs: [u8; 3],
    pub(crate) in_ep_addrs: [u8; 3],
}

pub(crate) fn build_audio_function<'d, D: Driver<'d>>(
    builder: &mut Builder<'d, D>,
) -> AudioEndpoints<'d, D> {
    let mut function = builder.function(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_CONTROL,
        USB_AUDIO_PROTOCOL_UNDEFINED,
    );

    let mut control_if = function.interface();
    let control_if_number = control_if.interface_number();
    let out_streaming_if_number = u8::from(control_if_number) + 1;
    let in_streaming_if_number = u8::from(control_if_number) + 2;
    let mut alt = control_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_CONTROL,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    write_audio_control_descriptors(&mut alt, out_streaming_if_number, in_streaming_if_number);

    let mut out_if = function.interface();
    let out_streaming_if = out_if.interface_number();

    let _alt0 = out_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );

    let mut alt16 = out_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    let bit_depth = BitDepth::Pcm16;
    let packet_size = max_packet_size(bit_depth);
    write_streaming_descriptors(&mut alt16, OUT_USB_TERMINAL_ID, bit_depth);
    let out_ep16 = alt16.alloc_endpoint_out(EndpointType::Isochronous, None, packet_size, 1);
    let out_info16 = *out_ep16.info();
    write_audio_data_endpoint(
        &mut alt16,
        &out_info16,
        packet_size,
        SynchronizationType::Adaptive,
    );
    write_class_specific_endpoint(&mut alt16);

    let mut alt24 = out_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    let bit_depth = BitDepth::Pcm24;
    let packet_size = max_packet_size(bit_depth);
    write_streaming_descriptors(&mut alt24, OUT_USB_TERMINAL_ID, bit_depth);
    let out_ep24 = alt24.alloc_endpoint_out(EndpointType::Isochronous, None, packet_size, 1);
    let out_info24 = *out_ep24.info();
    write_audio_data_endpoint(
        &mut alt24,
        &out_info24,
        packet_size,
        SynchronizationType::Adaptive,
    );
    write_class_specific_endpoint(&mut alt24);

    let mut alt32 = out_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    let bit_depth = BitDepth::Pcm32;
    let packet_size = max_packet_size(bit_depth);
    write_streaming_descriptors(&mut alt32, OUT_USB_TERMINAL_ID, bit_depth);
    let out_ep32 = alt32.alloc_endpoint_out(EndpointType::Isochronous, None, packet_size, 1);
    let out_info32 = *out_ep32.info();
    write_audio_data_endpoint(
        &mut alt32,
        &out_info32,
        packet_size,
        SynchronizationType::Adaptive,
    );
    write_class_specific_endpoint(&mut alt32);

    let mut in_if = function.interface();
    let in_streaming_if = in_if.interface_number();
    let _alt0 = in_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );

    let mut alt16 = in_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    let bit_depth = BitDepth::Pcm16;
    let packet_size = max_packet_size(bit_depth);
    write_streaming_descriptors(&mut alt16, IN_USB_TERMINAL_ID, bit_depth);
    let in_ep16 = alt16.alloc_endpoint_in(EndpointType::Isochronous, None, packet_size, 1);
    let in_info16 = *in_ep16.info();
    write_audio_data_endpoint(
        &mut alt16,
        &in_info16,
        packet_size,
        SynchronizationType::Asynchronous,
    );
    write_class_specific_endpoint(&mut alt16);

    let mut alt24 = in_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    let bit_depth = BitDepth::Pcm24;
    let packet_size = max_packet_size(bit_depth);
    write_streaming_descriptors(&mut alt24, IN_USB_TERMINAL_ID, bit_depth);
    let in_ep24 = alt24.alloc_endpoint_in(EndpointType::Isochronous, None, packet_size, 1);
    let in_info24 = *in_ep24.info();
    write_audio_data_endpoint(
        &mut alt24,
        &in_info24,
        packet_size,
        SynchronizationType::Asynchronous,
    );
    write_class_specific_endpoint(&mut alt24);

    let mut alt32 = in_if.alt_setting(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_STREAMING,
        USB_AUDIO_PROTOCOL_UNDEFINED,
        None,
    );
    let bit_depth = BitDepth::Pcm32;
    let packet_size = max_packet_size(bit_depth);
    write_streaming_descriptors(&mut alt32, IN_USB_TERMINAL_ID, bit_depth);
    let in_ep32 = alt32.alloc_endpoint_in(EndpointType::Isochronous, None, packet_size, 1);
    let in_info32 = *in_ep32.info();
    write_audio_data_endpoint(
        &mut alt32,
        &in_info32,
        packet_size,
        SynchronizationType::Asynchronous,
    );
    write_class_specific_endpoint(&mut alt32);

    AudioEndpoints {
        out_ep16,
        out_ep24,
        out_ep32,
        in_ep16,
        in_ep24,
        in_ep32,
        out_streaming_if,
        in_streaming_if,
        out_ep_addrs: [
            u8::from(out_info16.addr),
            u8::from(out_info24.addr),
            u8::from(out_info32.addr),
        ],
        in_ep_addrs: [
            u8::from(in_info16.addr),
            u8::from(in_info24.addr),
            u8::from(in_info32.addr),
        ],
    }
}

fn write_audio_control_descriptors<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    out_streaming_if: u8,
    in_streaming_if: u8,
) {
    const AC_TOTAL_LENGTH: u16 = 52;
    let [lo, hi] = AC_TOTAL_LENGTH.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_HEADER,
            0x00,
            0x01,
            lo,
            hi,
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
    let [tt0, tt1] = terminal_type.to_le_bytes();
    let [cc0, cc1] = channel_config.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_INPUT_TERMINAL,
            terminal_id,
            tt0,
            tt1,
            0x00,
            channel_count,
            cc0,
            cc1,
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
    let [lo, hi] = terminal_type.to_le_bytes();

    alt.descriptor(
        CS_INTERFACE,
        &[
            AC_OUTPUT_TERMINAL,
            terminal_id,
            lo,
            hi,
            0x00,
            source_id,
            0x00,
        ],
    );
}

fn write_streaming_descriptors<'d, D: Driver<'d>>(
    alt: &mut InterfaceAltBuilder<'_, 'd, D>,
    terminal_link: u8,
    bit_depth: BitDepth,
) {
    let [lo, hi] = FORMAT_PCM.to_le_bytes();
    let sample_rates = supported_sample_rates(bit_depth);

    alt.descriptor(CS_INTERFACE, &[AS_GENERAL, terminal_link, 0x01, lo, hi]);

    let mut format_type = [0u8; 18];
    format_type[0] = FORMAT_TYPE;
    format_type[1] = FORMAT_TYPE_I;
    format_type[2] = CHANNEL_COUNT;
    format_type[3] = subframe_size(bit_depth);
    format_type[4] = bit_resolution(bit_depth);
    format_type[5] = u8::try_from(sample_rates.len()).expect("sample rate count must fit u8");
    let mut offset = 6;
    for rate in sample_rates {
        let [rate_hz @ .., 0] = rate.hz().to_le_bytes() else {
            unreachable!("sample rate must fit UAC1 24-bit tSamFreq");
        };
        format_type[offset..offset + 3].copy_from_slice(&rate_hz);
        offset += 3;
    }
    alt.descriptor(CS_INTERFACE, &format_type[..offset]);
}

const fn max_packet_size(bit_depth: BitDepth) -> u16 {
    match bit_depth {
        BitDepth::Pcm16 | BitDepth::Pcm24 => {
            97 * CHANNEL_COUNT as u16 * subframe_size(bit_depth) as u16
        }
        BitDepth::Pcm32 => 49 * CHANNEL_COUNT as u16 * subframe_size(bit_depth) as u16,
    }
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
