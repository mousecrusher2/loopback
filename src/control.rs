use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::types::InterfaceNumber;

use crate::descriptors::AudioRouting;
use crate::spec::{self, PcmFormat, SampleRate, StreamDirection};
use crate::streams::{AudioQueue, StreamState};

const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;
const SAMPLING_FREQ_CONTROL: u8 = 0x01;

pub(crate) struct AudioControl {
    state: &'static StreamState,
    queue: &'static AudioQueue,
    routing: AudioRouting,
}

impl AudioControl {
    pub(crate) const fn new(
        state: &'static StreamState,
        queue: &'static AudioQueue,
        routing: AudioRouting,
    ) -> Self {
        Self {
            state,
            queue,
            routing,
        }
    }

    fn direction_for_interface(&self, interface: InterfaceNumber) -> Option<StreamDirection> {
        if interface == self.routing.playback_interface {
            Some(StreamDirection::Playback)
        } else if interface == self.routing.capture_interface {
            Some(StreamDirection::Capture)
        } else {
            None
        }
    }

    fn endpoint_format(
        &self,
        endpoint_address: u8,
    ) -> Option<(StreamDirection, &'static PcmFormat)> {
        if let Some(slot) = self
            .routing
            .playback_endpoints
            .iter()
            .position(|addr| *addr == endpoint_address)
        {
            return spec::format_by_endpoint_slot(slot)
                .map(|format| (StreamDirection::Playback, format));
        }

        self.routing
            .capture_endpoints
            .iter()
            .position(|addr| *addr == endpoint_address)
            .and_then(spec::format_by_endpoint_slot)
            .map(|format| (StreamDirection::Capture, format))
    }
}

impl embassy_usb::Handler for AudioControl {
    fn reset(&mut self) {
        self.state.reset();
        self.queue.clear();
    }

    fn set_alternate_setting(&mut self, interface: InterfaceNumber, alternate_setting: u8) {
        if let Some(direction) = self.direction_for_interface(interface) {
            let _ = self
                .state
                .set_alternate_setting(direction, alternate_setting);
            self.queue.clear();
        }
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        let endpoint_address = (req.index & 0xff) as u8;
        let control_selector = (req.value >> 8) as u8;

        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Endpoint
            || req.request != SET_CUR
            || control_selector != SAMPLING_FREQ_CONTROL
        {
            return None;
        }

        let (direction, format) = self.endpoint_format(endpoint_address)?;
        let Some(rate) = requested_rate(data) else {
            return Some(OutResponse::Rejected);
        };
        if !format.supports(rate) || !self.state.set_rate(direction, rate) {
            return Some(OutResponse::Rejected);
        }

        self.queue.clear();
        Some(OutResponse::Accepted)
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        let endpoint_address = (req.index & 0xff) as u8;
        let control_selector = (req.value >> 8) as u8;

        if req.request_type != RequestType::Class
            || req.recipient != Recipient::Endpoint
            || control_selector != SAMPLING_FREQ_CONTROL
        {
            return None;
        }

        let (direction, endpoint_format) = self.endpoint_format(endpoint_address)?;
        let current_rate = self.state.snapshot().stream(direction).rate();
        let response = match req.request {
            GET_CUR => spec::rate_or_default_for_format(current_rate, endpoint_format).hz(),
            GET_MIN => SampleRate::R44100.hz(),
            GET_MAX => endpoint_format.max_rate().hz(),
            GET_RES => 1,
            _ => return Some(InResponse::Rejected),
        };

        let Some(out) = buf.first_chunk_mut::<3>() else {
            return Some(InResponse::Rejected);
        };
        let [b0, b1, b2, _] = response.to_le_bytes();
        *out = [b0, b1, b2];
        Some(InResponse::Accepted(out))
    }
}

fn requested_rate(data: &[u8]) -> Option<SampleRate> {
    let [b0, b1, b2] = *data.first_chunk::<3>()?;
    if data.len() != 3 {
        return None;
    }
    let hz = u32::from(b0) | (u32::from(b1) << 8) | (u32::from(b2) << 16);
    SampleRate::from_hz(hz)
}
