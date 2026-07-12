use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::zerocopy_channel::{Channel, Receiver, Sender};
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use heapless::Vec;
use static_cell::{ConstStaticCell, StaticCell};

use crate::diagnostics;
use crate::spec::{
    self, DEFAULT_RATE, DuplexSelection, SampleRate, StreamDirection, StreamSelection,
};

pub(crate) type AudioPacket = Vec<u8, { spec::MAX_AUDIO_PACKET_BYTES }>;

// A queue per format keeps every grant tied to one frame width and endpoint MPS.
// NoopRawMutex is valid because both halves stay on the same thread-mode executor;
// no interrupt handler or second core accesses these channels.
// See docs/uac1-design.md for the transition and clearing policy.
pub(crate) type AudioQueue = Channel<'static, NoopRawMutex, AudioPacket>;
pub(crate) type AudioQueues = [AudioQueue; spec::FORMAT_COUNT];
pub(crate) type AudioSender = Sender<'static, NoopRawMutex, AudioPacket>;
pub(crate) type AudioReceiver = Receiver<'static, NoopRawMutex, AudioPacket>;

pub(crate) fn init_audio_queues() -> &'static mut AudioQueues {
    static PACKETS: ConstStaticCell<
        [[AudioPacket; spec::PACKET_QUEUE_CAPACITY]; spec::FORMAT_COUNT],
    > = ConstStaticCell::new(
        [const { [const { AudioPacket::new() }; spec::PACKET_QUEUE_CAPACITY] }; spec::FORMAT_COUNT],
    );
    static QUEUES: StaticCell<AudioQueues> = StaticCell::new();

    let packet_queues = PACKETS.take();
    QUEUES.init(
        packet_queues
            .each_mut()
            .map(|packets| AudioQueue::new(&mut packets[..])),
    )
}

pub(crate) struct StreamState {
    encoded: AtomicU32,
}

impl StreamState {
    pub(crate) const fn new() -> Self {
        Self {
            encoded: AtomicU32::new(DuplexSelection::inactive().encode()),
        }
    }

    pub(crate) fn reset(&self) {
        self.encoded
            .store(DuplexSelection::inactive().encode(), Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> DuplexSelection {
        DuplexSelection::decode(self.encoded.load(Ordering::Relaxed))
    }

    pub(crate) fn set_alternate_setting(
        &self,
        direction: StreamDirection,
        alternate_setting: u8,
    ) -> bool {
        if alternate_setting != 0 && spec::format_by_alternate_setting(alternate_setting).is_none()
        {
            return false;
        }

        self.update(direction, |current| {
            let selected = StreamSelection::new(alternate_setting, current.rate());
            Some(selected.format().map_or(selected, |format| {
                StreamSelection::new(
                    alternate_setting,
                    spec::rate_or_default_for_format(current.rate(), format),
                )
            }))
        })
    }

    pub(crate) fn set_rate(&self, direction: StreamDirection, rate: SampleRate) -> bool {
        self.update(direction, |current| {
            if current.format().is_none_or(|format| format.supports(rate)) {
                Some(StreamSelection::new(current.alternate_setting(), rate))
            } else {
                None
            }
        })
    }

    fn update(
        &self,
        direction: StreamDirection,
        mut select: impl FnMut(StreamSelection) -> Option<StreamSelection>,
    ) -> bool {
        self.encoded
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                let selection = DuplexSelection::decode(bits);
                let stream = select(selection.stream(direction))?;
                Some(selection.with_stream(direction, stream).encode())
            })
            .is_ok()
    }
}

pub(crate) async fn playback_task<'d, D: Driver<'d>>(
    slot: usize,
    mut endpoint: D::EndpointOut,
    state: &'static StreamState,
    mut sender: AudioSender,
) {
    let format =
        spec::format_by_endpoint_slot(slot).expect("playback task has a valid format slot");
    let alternate_setting =
        spec::alternate_setting_for_slot(slot).expect("playback task has an alternate setting");
    let max_packet_size = format.max_packet_size() as usize;
    let mut discard = [0_u8; spec::MAX_AUDIO_PACKET_BYTES];

    // Endpoint I/O is deliberately allowed to finish across a state change: the
    // generic endpoint traits do not specify their post-cancellation state.
    loop {
        endpoint.wait_enabled().await;
        loop {
            let selection = state.snapshot();
            if selection.playback.alternate_setting() != alternate_setting {
                break;
            }

            if !selection.loopback_enabled() {
                match endpoint.read(&mut discard[..max_packet_size]).await {
                    Ok(_) => {}
                    Err(EndpointError::Disabled) => break,
                    Err(EndpointError::BufferOverflow) => return,
                }
                continue;
            }

            // Isochronous OUT has no retry/backpressure mechanism. Keep servicing
            // the endpoint and drop the newest packet when all grants are busy.
            let Some(packet) = sender.try_send() else {
                match endpoint.read(&mut discard[..max_packet_size]).await {
                    Ok(_) => {}
                    Err(EndpointError::Disabled) => break,
                    Err(EndpointError::BufferOverflow) => return,
                }
                continue;
            };

            packet
                .resize(max_packet_size, 0)
                .expect("format MPS fits the audio packet");
            match endpoint.read(packet.as_mut_slice()).await {
                Ok(len) => {
                    // Do not publish a grant completed while loopback was being
                    // disabled or this alternate was being deselected.
                    let selection = state.snapshot();
                    if selection.playback.alternate_setting() != alternate_setting {
                        break;
                    }
                    if selection.loopback_enabled()
                        && len != 0
                        && len.is_multiple_of(format.audio_frame_bytes())
                    {
                        packet.truncate(len);
                        sender.send_done();
                    }
                }
                Err(EndpointError::Disabled) => break,
                Err(EndpointError::BufferOverflow) => return,
            }
        }
    }
}

pub(crate) async fn capture_task<'d, D: Driver<'d>>(
    slot: usize,
    mut endpoint: D::EndpointIn,
    state: &'static StreamState,
    mut receiver: AudioReceiver,
) {
    let format = spec::format_by_endpoint_slot(slot).expect("capture task has a valid format slot");
    let alternate_setting =
        spec::alternate_setting_for_slot(slot).expect("capture task has an alternate setting");
    let max_packet_size = format.max_packet_size() as usize;
    let mut silence = [0_u8; spec::MAX_AUDIO_PACKET_BYTES];
    let mut clock = PacketClock::new();

    // As in playback_task, never cancel an in-flight endpoint operation merely
    // because a control request changed the selected alternate or rate.
    loop {
        endpoint.wait_enabled().await;
        loop {
            let selection = state.snapshot();
            if selection.capture.alternate_setting() != alternate_setting {
                drain_packets(&mut receiver, max_packet_size);
                break;
            }

            if selection.loopback_enabled()
                && let Some(packet) = receiver.try_receive()
            {
                let write_result = endpoint.write(packet.as_slice()).await;
                // Vec length is also the next OUT read buffer length. Restore the
                // full format MPS before returning this zero-copy slot.
                packet
                    .resize(max_packet_size, 0)
                    .expect("format MPS fits the audio packet");
                receiver.receive_done();
                match write_result {
                    Ok(()) => continue,
                    Err(EndpointError::Disabled) => break,
                    Err(EndpointError::BufferOverflow) => return,
                }
            }

            if !selection.loopback_enabled() {
                drain_packets(&mut receiver, max_packet_size);
            }

            let len = clock.next_len(selection.capture.rate(), format.audio_frame_bytes());
            diagnostics::record_in_silence();
            silence[..len].fill(0);
            match endpoint.write(&silence[..len]).await {
                Ok(()) => {}
                Err(EndpointError::Disabled) => break,
                Err(EndpointError::BufferOverflow) => return,
            }
        }
    }
}

fn drain_packets(receiver: &mut AudioReceiver, max_packet_size: usize) {
    // clear() can reset indices while the other half owns a grant across await.
    // Drain only committed receiver grants and leave uncommitted grants alone.
    while let Some(packet) = receiver.try_receive() {
        packet
            .resize(max_packet_size, 0)
            .expect("format MPS fits the audio packet");
        receiver.receive_done();
    }
}

struct PacketClock {
    rate: SampleRate,
    frame_bytes: usize,
    accumulator: u32,
}

impl PacketClock {
    const fn new() -> Self {
        Self {
            rate: DEFAULT_RATE,
            frame_bytes: 0,
            accumulator: 0,
        }
    }

    fn next_len(&mut self, rate: SampleRate, frame_bytes: usize) -> usize {
        if self.rate != rate || self.frame_bytes != frame_bytes {
            self.rate = rate;
            self.frame_bytes = frame_bytes;
            self.accumulator = 0;
        }

        self.accumulator += rate.hz();
        let frames = self.accumulator / 1_000;
        self.accumulator %= 1_000;
        frames as usize * frame_bytes
    }
}

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::{PacketClock, StreamState};
    use crate::spec::{
        SampleRate, StreamDirection, alternate_setting_for_slot, format_by_endpoint_slot,
    };

    #[test]
    fn unsupported_rate_does_not_change_state() {
        let state = StreamState::new();
        state.set_alternate_setting(
            StreamDirection::Capture,
            alternate_setting_for_slot(2).unwrap(),
        );
        let before = state.snapshot();

        assert!(!state.set_rate(StreamDirection::Capture, SampleRate::R96000));
        assert_eq!(state.snapshot(), before);
    }

    #[test]
    fn packet_clock_emits_fractional_cadence() {
        let format = format_by_endpoint_slot(0).unwrap();
        let frame_bytes = format.audio_frame_bytes();
        let mut clock = PacketClock::new();
        let mut total = 0;

        for index in 0..10 {
            let len = clock.next_len(SampleRate::R44100, frame_bytes);
            total += len;
            assert_eq!(len, if index == 9 { 45 } else { 44 } * frame_bytes);
        }

        assert_eq!(total, 441 * frame_bytes);
    }
}
