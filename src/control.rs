use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::types::InterfaceNumber;

use crate::audio::{AudioState, AudioStreamingAlternateSetting, SampleRate, StreamDirection};
use crate::diag;
use crate::tasks::{AudioPipe, PacketLenQueue};

const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;
const SAMPLING_FREQ_CONTROL: u8 = 0x01;

pub struct AudioControlHandler {
    state: &'static AudioState,
    pipe: &'static AudioPipe,
    packet_lens: &'static PacketLenQueue,
    out_streaming_if: InterfaceNumber,
    in_streaming_if: InterfaceNumber,
    out_ep_addrs: [u8; 3],
    in_ep_addrs: [u8; 3],
}

impl AudioControlHandler {
    pub fn new(
        state: &'static AudioState,
        pipe: &'static AudioPipe,
        packet_lens: &'static PacketLenQueue,
        out_streaming_if: InterfaceNumber,
        in_streaming_if: InterfaceNumber,
        out_ep_addrs: [u8; 3],
        in_ep_addrs: [u8; 3],
    ) -> Self {
        Self {
            state,
            pipe,
            packet_lens,
            out_streaming_if,
            in_streaming_if,
            out_ep_addrs,
            in_ep_addrs,
        }
    }

    fn endpoint_stream(
        &self,
        ep_addr: u8,
    ) -> Option<(StreamDirection, AudioStreamingAlternateSetting)> {
        const ALTERNATE_SETTINGS: [AudioStreamingAlternateSetting; 3] = [
            AudioStreamingAlternateSetting::Pcm16,
            AudioStreamingAlternateSetting::Pcm24,
            AudioStreamingAlternateSetting::Pcm32,
        ];

        if let Some(index) = self.out_ep_addrs.iter().position(|addr| *addr == ep_addr) {
            Some((StreamDirection::Out, ALTERNATE_SETTINGS[index]))
        } else {
            self.in_ep_addrs
                .iter()
                .position(|addr| *addr == ep_addr)
                .map(|index| (StreamDirection::In, ALTERNATE_SETTINGS[index]))
        }
    }

    fn direction_for_interface(&self, iface: InterfaceNumber) -> Option<StreamDirection> {
        if iface == self.out_streaming_if {
            Some(StreamDirection::Out)
        } else if iface == self.in_streaming_if {
            Some(StreamDirection::In)
        } else {
            None
        }
    }
}

impl embassy_usb::Handler for AudioControlHandler {
    fn reset(&mut self) {
        self.state.reset();
        diag::set_out_alt(0);
        diag::set_in_alt(0);
        diag::set_out_rate(0);
        diag::set_in_rate(0);
        self.pipe.clear();
        self.packet_lens.clear();
    }

    fn set_alternate_setting(&mut self, iface: InterfaceNumber, alternate_setting: u8) {
        if let Some(direction) = self.direction_for_interface(iface) {
            let Some(alternate_setting) =
                AudioStreamingAlternateSetting::from_number(alternate_setting)
            else {
                self.pipe.clear();
                self.packet_lens.clear();
                return;
            };

            let format = self
                .state
                .set_alternate_setting(direction, alternate_setting);
            match direction {
                StreamDirection::Out => {
                    diag::set_out_alt(format.alternate_setting.number());
                    diag::set_out_rate(format.rate.hz());
                }
                StreamDirection::In => {
                    diag::set_in_alt(format.alternate_setting.number());
                    diag::set_in_rate(format.rate.hz());
                }
            }
            self.pipe.clear();
            self.packet_lens.clear();
        }
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        let ep_addr = (req.index & 0xff) as u8;
        let control_selector = (req.value >> 8) as u8;

        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Endpoint
            || req.request != SET_CUR
            || control_selector != SAMPLING_FREQ_CONTROL
        {
            return None;
        }

        let (direction, alt) = self.endpoint_stream(ep_addr)?;

        if data.len() != 3 {
            return Some(OutResponse::Rejected);
        }

        let requested = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
        let Some(rate) = SampleRate::from_hz(requested) else {
            return Some(OutResponse::Rejected);
        };
        if !alt.supports_sample_rate(rate) {
            return Some(OutResponse::Rejected);
        }

        let Some(format) = self.state.set_rate(direction, rate) else {
            return Some(OutResponse::Rejected);
        };
        match direction {
            StreamDirection::Out => diag::set_out_rate(format.rate.hz()),
            StreamDirection::In => diag::set_in_rate(format.rate.hz()),
        }
        self.pipe.clear();
        self.packet_lens.clear();
        Some(OutResponse::Accepted)
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        let ep_addr = (req.index & 0xff) as u8;
        let control_selector = (req.value >> 8) as u8;

        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Endpoint
            || control_selector != SAMPLING_FREQ_CONTROL
        {
            return None;
        }

        let (direction, alt) = self.endpoint_stream(ep_addr)?;

        let formats = self.state.formats();
        let current_rate = match direction {
            StreamDirection::Out => formats.out.rate,
            StreamDirection::In => formats.in_.rate,
        };

        let value = match req.request {
            GET_CUR => alt.sample_rate_or_default(current_rate).hz(),
            GET_MIN => 44_100,
            GET_MAX if alt == AudioStreamingAlternateSetting::Pcm32 => 48_000,
            GET_MAX => 96_000,
            GET_RES => 1,
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
