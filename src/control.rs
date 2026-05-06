use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::types::InterfaceNumber;

use crate::audio::{AudioState, StreamDirection, closest_supported_rate};
use crate::tasks::AudioPipe;

const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;
const SAMPLING_FREQ_CONTROL: u8 = 0x01;

pub struct AudioControlHandler {
    state: &'static AudioState,
    pipe: &'static AudioPipe,
    out_streaming_if: InterfaceNumber,
    in_streaming_if: InterfaceNumber,
    out_ep_addr: u8,
    in_ep_addr: u8,
}

impl AudioControlHandler {
    pub fn new(
        state: &'static AudioState,
        pipe: &'static AudioPipe,
        out_streaming_if: InterfaceNumber,
        in_streaming_if: InterfaceNumber,
        out_ep_addr: u8,
        in_ep_addr: u8,
    ) -> Self {
        Self {
            state,
            pipe,
            out_streaming_if,
            in_streaming_if,
            out_ep_addr,
            in_ep_addr,
        }
    }

    fn direction_for_endpoint(&self, ep_addr: u8) -> Option<StreamDirection> {
        if ep_addr == self.out_ep_addr {
            Some(StreamDirection::Out)
        } else if ep_addr == self.in_ep_addr {
            Some(StreamDirection::In)
        } else {
            None
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
        self.pipe.clear();
    }

    fn set_alternate_setting(&mut self, iface: InterfaceNumber, alternate_setting: u8) {
        if let Some(direction) = self.direction_for_interface(iface) {
            self.state.set_alt(direction, alternate_setting);
            self.pipe.clear();
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

        let direction = self.direction_for_endpoint(ep_addr)?;

        if data.len() != 3 {
            return Some(OutResponse::Rejected);
        }

        let requested = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
        self.state
            .set_rate_hz(direction, closest_supported_rate(requested));
        self.pipe.clear();
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

        let direction = self.direction_for_endpoint(ep_addr)?;

        let value = match req.request {
            GET_CUR => self.state.rate_hz(direction),
            GET_MIN => 44_100,
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
