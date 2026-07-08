use core::sync::atomic::{AtomicU32, Ordering};

use embassy_futures::yield_now;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use heapless::Vec;

use crate::diagnostics;
use crate::spec::{
    self, DEFAULT_RATE, DuplexSelection, SampleRate, StreamDirection, StreamSelection,
};

pub(crate) type AudioPacket = Vec<u8, { spec::MAX_AUDIO_PACKET_BYTES }>;
pub(crate) type AudioQueue =
    Channel<CriticalSectionRawMutex, AudioPacket, { spec::PACKET_QUEUE_CAPACITY }>;

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
            selected.format().map_or(selected, |format| {
                StreamSelection::new(
                    alternate_setting,
                    spec::rate_or_default_for_format(current.rate(), format),
                )
            })
        });
        true
    }

    pub(crate) fn set_rate(&self, direction: StreamDirection, rate: SampleRate) -> bool {
        let mut accepted = false;
        self.update(direction, |current| {
            accepted = current.format().is_none_or(|format| format.supports(rate));
            if accepted {
                StreamSelection::new(current.alternate_setting(), rate)
            } else {
                current
            }
        });
        accepted
    }

    fn update(
        &self,
        direction: StreamDirection,
        mut select: impl FnMut(StreamSelection) -> StreamSelection,
    ) {
        let _ = self
            .encoded
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                let selection = DuplexSelection::decode(bits);
                let stream = select(selection.stream(direction));
                Some(selection.with_stream(direction, stream).encode())
            });
    }
}

pub(crate) async fn playback_task<'d, D: Driver<'d>>(
    mut endpoint: D::EndpointOut,
    state: &'static StreamState,
    queue: &'static AudioQueue,
) {
    let mut packet = [0_u8; spec::MAX_AUDIO_PACKET_BYTES];

    loop {
        endpoint.wait_enabled().await;

        loop {
            match endpoint.read(&mut packet).await {
                Ok(0) | Err(EndpointError::BufferOverflow) => {}
                Ok(len) => accept_playback_packet(state, queue, &packet[..len]),
                Err(EndpointError::Disabled) => {
                    queue.clear();
                    break;
                }
            }
        }
    }
}

fn accept_playback_packet(state: &StreamState, queue: &AudioQueue, packet: &[u8]) {
    let selection = state.snapshot();
    let Some(format) = selection.playback.format() else {
        queue.clear();
        return;
    };

    if !selection.loopback_enabled() || !packet.len().is_multiple_of(format.audio_frame_bytes()) {
        queue.clear();
        return;
    }

    if enqueue(queue, packet).is_err() {
        queue.clear();
        let _ = enqueue(queue, packet);
    }
}

fn enqueue(queue: &AudioQueue, packet: &[u8]) -> Result<(), ()> {
    let mut owned = AudioPacket::new();
    owned.extend_from_slice(packet).map_err(|_| ())?;
    queue.try_send(owned).map_err(|_| ())
}

pub(crate) async fn capture_task<'d, D: Driver<'d>>(
    mut endpoint: D::EndpointIn,
    state: &'static StreamState,
    queue: &'static AudioQueue,
) {
    let mut silence = [0_u8; spec::MAX_AUDIO_PACKET_BYTES];
    let mut clock = PacketClock::new();

    loop {
        endpoint.wait_enabled().await;

        loop {
            let selection = state.snapshot();
            let capture = selection.capture;
            let Some(format) = capture.format() else {
                yield_now().await;
                continue;
            };

            let write_result = if selection.loopback_enabled() {
                if let Ok(packet) = queue.try_receive() {
                    endpoint.write(packet.as_slice()).await
                } else {
                    let len = clock.next_len(capture.rate(), format.audio_frame_bytes());
                    diagnostics::record_in_silence();
                    silence[..len].fill(0);
                    endpoint.write(&silence[..len]).await
                }
            } else {
                queue.clear();
                let len = clock.next_len(capture.rate(), format.audio_frame_bytes());
                diagnostics::record_in_silence();
                silence[..len].fill(0);
                endpoint.write(&silence[..len]).await
            };

            match write_result {
                Err(EndpointError::Disabled) => {
                    queue.clear();
                    break;
                }
                Ok(()) | Err(EndpointError::BufferOverflow) => {}
            }
        }
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
