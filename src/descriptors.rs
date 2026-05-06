use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{Driver, Endpoint, EndpointInfo, EndpointType};
use embassy_usb::types::InterfaceNumber;
use embassy_usb::{Builder, InterfaceAltBuilder};

use crate::audio::{CHANNEL_COUNT, MAX_PACKET_SIZE, MAX_PACKET_SIZE_16, MAX_PACKET_SIZE_24};

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

pub struct AudioEndpoints<'d, D: Driver<'d>> {
    pub out_ep: D::EndpointOut,
    pub in_ep: D::EndpointIn,
    pub out_streaming_if: InterfaceNumber,
    pub in_streaming_if: InterfaceNumber,
    pub out_ep_addr: u8,
    pub in_ep_addr: u8,
}

pub fn build_audio_function<'d, D: Driver<'d>>(
    builder: &mut Builder<'d, D>,
) -> AudioEndpoints<'d, D> {
    let mut function = builder.function(
        USB_CLASS_AUDIO,
        USB_SUBCLASS_AUDIO_CONTROL,
        USB_AUDIO_PROTOCOL_UNDEFINED,
    );

    {
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
    }

    let out_ep;
    let out_info;
    let out_streaming_if;
    {
        let mut out_if = function.interface();
        out_streaming_if = out_if.interface_number();
        {
            let _alt0 = out_if.alt_setting(
                USB_CLASS_AUDIO,
                USB_SUBCLASS_AUDIO_STREAMING,
                USB_AUDIO_PROTOCOL_UNDEFINED,
                None,
            );
        }

        {
            let mut alt16 = out_if.alt_setting(
                USB_CLASS_AUDIO,
                USB_SUBCLASS_AUDIO_STREAMING,
                USB_AUDIO_PROTOCOL_UNDEFINED,
                None,
            );
            write_streaming_descriptors(&mut alt16, OUT_USB_TERMINAL_ID, 2, 16);
            out_ep = alt16.alloc_endpoint_out(
                EndpointType::Isochronous,
                None,
                MAX_PACKET_SIZE as u16,
                1,
            );
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
    }

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
