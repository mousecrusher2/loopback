use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::driver::EndpointAddress;
use embassy_usb::types::InterfaceNumber;

use crate::descriptors::AudioControlMap;
use crate::spec::{self, PcmFormat, SampleRate, StreamDirection};
use crate::streams::{EndpointRate, EndpointRateWatch, EndpointRateWatches};

const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;
const SAMPLING_FREQ_CONTROL: u8 = 0x01;

pub(crate) struct AudioControl {
    rates: &'static EndpointRateWatches,
    control_map: AudioControlMap,
}

impl AudioControl {
    pub(crate) const fn new(
        rates: &'static EndpointRateWatches,
        control_map: AudioControlMap,
    ) -> Self {
        Self { rates, control_map }
    }

    fn direction_for_interface(&self, interface: InterfaceNumber) -> Option<StreamDirection> {
        if interface == self.control_map.playback_interface {
            Some(StreamDirection::Playback)
        } else if interface == self.control_map.capture_interface {
            Some(StreamDirection::Capture)
        } else {
            None
        }
    }

    fn endpoint_format(
        &self,
        endpoint_address: EndpointAddress,
    ) -> Option<(&EndpointRateWatch, &'static PcmFormat)> {
        if let Some(slot) = self
            .control_map
            .playback_endpoint_addresses
            .iter()
            .position(|addr| *addr == endpoint_address)
        {
            return Some((
                self.rates.watch(StreamDirection::Playback, slot)?,
                spec::format_by_endpoint_slot(slot)?,
            ));
        }

        let slot = self
            .control_map
            .capture_endpoint_addresses
            .iter()
            .position(|addr| *addr == endpoint_address)?;
        Some((
            self.rates.watch(StreamDirection::Capture, slot)?,
            spec::format_by_endpoint_slot(slot)?,
        ))
    }
}

fn endpoint_address(index: u16) -> Option<EndpointAddress> {
    let [address, entity_id] = index.to_le_bytes();
    (entity_id == 0).then(|| EndpointAddress::from(address))
}

fn is_sampling_frequency_control(value: u16) -> bool {
    let [reserved, control_selector] = value.to_le_bytes();
    reserved == 0 && control_selector == SAMPLING_FREQ_CONTROL
}

impl embassy_usb::Handler for AudioControl {
    fn reset(&mut self) {
        self.rates.reset();
    }

    fn set_alternate_setting(&mut self, interface: InterfaceNumber, alternate_setting: u8) {
        let Some(direction) = self.direction_for_interface(interface) else {
            return;
        };
        let Some(slot) = spec::format_slot_by_alternate_setting(alternate_setting) else {
            return;
        };
        let _ = self.rates.notify(direction, slot);
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        if req.request_type != RequestType::Class || req.recipient != Recipient::Endpoint {
            return None;
        }

        let Some(endpoint_address) = endpoint_address(req.index) else {
            return Some(OutResponse::Rejected);
        };
        let (rate_watch, format) = self.endpoint_format(endpoint_address)?;
        if req.request != SET_CUR || !is_sampling_frequency_control(req.value) {
            return Some(OutResponse::Rejected);
        }
        let Some(requested_hz) = requested_frequency_hz(data) else {
            return Some(OutResponse::Rejected);
        };
        let rate = closest_supported_rate(format, requested_hz);
        rate_watch.sender().send(EndpointRate::Configured(rate));

        Some(OutResponse::Accepted)
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        if req.request_type != RequestType::Class || req.recipient != Recipient::Endpoint {
            return None;
        }

        let Some(endpoint_address) = endpoint_address(req.index) else {
            return Some(InResponse::Rejected);
        };
        let (rate_watch, endpoint_format) = self.endpoint_format(endpoint_address)?;
        if !is_sampling_frequency_control(req.value) {
            return Some(InResponse::Rejected);
        }
        let Some(out) = buf.first_chunk_mut::<3>() else {
            return Some(InResponse::Rejected);
        };
        let response = match req.request {
            GET_CUR => get_or_configure_rate(rate_watch, endpoint_format.default_rate()).hz(),
            GET_MIN => endpoint_format.min_rate().hz(),
            GET_MAX => endpoint_format.max_rate().hz(),
            GET_RES => 1,
            _ => return Some(InResponse::Rejected),
        };

        let [b0, b1, b2, _] = response.to_le_bytes();
        *out = [b0, b1, b2];
        Some(InResponse::Accepted(out))
    }
}

fn get_or_configure_rate(rate_watch: &EndpointRateWatch, default: SampleRate) -> SampleRate {
    match rate_watch
        .try_get()
        .expect("endpoint rate Watch is initialized")
    {
        EndpointRate::Configured(rate) => rate,
        EndpointRate::Unset => {
            rate_watch.sender().send(EndpointRate::Configured(default));
            default
        }
    }
}

fn requested_frequency_hz(data: &[u8]) -> Option<u32> {
    let &[b0, b1, b2] = data else {
        return None;
    };
    Some(u32::from(b0) | (u32::from(b1) << 8) | (u32::from(b2) << 16))
}

fn closest_supported_rate(format: &PcmFormat, requested_hz: u32) -> SampleRate {
    if let Some(rate) = SampleRate::from_hz(requested_hz)
        && format.supports(rate)
    {
        return rate;
    }

    format
        .rates
        .iter()
        .copied()
        .min_by_key(|rate| (rate.hz().abs_diff(requested_hz), rate.hz()))
        .expect("each advertised PCM format has at least one sample rate")
}

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use embassy_usb::driver::EndpointAddress;

    use super::{
        closest_supported_rate, endpoint_address, get_or_configure_rate,
        is_sampling_frequency_control, requested_frequency_hz,
    };
    use crate::spec::{SampleRate, format_by_endpoint_slot};
    use crate::streams::{EndpointRate, EndpointRateWatch};

    #[test]
    fn get_cur_sets_default_only_while_unset() {
        let rate_watch = EndpointRateWatch::new_with(EndpointRate::Unset);

        assert_eq!(
            get_or_configure_rate(&rate_watch, SampleRate::R48000),
            SampleRate::R48000
        );
        assert_eq!(
            rate_watch.try_get(),
            Some(EndpointRate::Configured(SampleRate::R48000))
        );
        assert_eq!(
            get_or_configure_rate(&rate_watch, SampleRate::R44100),
            SampleRate::R48000
        );
    }

    #[test]
    fn requested_frequency_is_rounded_to_the_closest_advertised_rate() {
        let format = format_by_endpoint_slot(2).unwrap();

        assert_eq!(closest_supported_rate(format, 48_000), SampleRate::R48000);
        assert_eq!(closest_supported_rate(format, 47_999), SampleRate::R48000);
        assert_eq!(closest_supported_rate(format, 88_200), SampleRate::R48000);
        assert_eq!(closest_supported_rate(format, 1), SampleRate::R44100);
        assert_eq!(closest_supported_rate(format, 46_050), SampleRate::R44100);
    }

    #[test]
    fn endpoint_control_fields_are_validated() {
        assert_eq!(endpoint_address(0x0081), Some(EndpointAddress::from(0x81)));
        assert_eq!(endpoint_address(0x0181), None);

        assert!(is_sampling_frequency_control(0x0100));
        assert!(!is_sampling_frequency_control(0x0101));
        assert!(!is_sampling_frequency_control(0x0200));
    }

    #[test]
    fn sampling_frequency_parameter_must_be_exactly_three_bytes() {
        assert_eq!(requested_frequency_hz(&[0x80, 0xbb, 0x00]), Some(48_000));
        assert_eq!(requested_frequency_hz(&[0x80, 0xbb]), None);
        assert_eq!(requested_frequency_hz(&[0x80, 0xbb, 0x00, 0x00]), None);
    }
}
