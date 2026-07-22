use core::cell::RefCell;

use embassy_sync::blocking_mutex::ThreadModeMutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::watch::{Receiver as WatchReceiver, Watch};
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use heapless::{Deque, Vec};
use static_cell::StaticCell;

use crate::diagnostics;
use crate::spec::{self, DEFAULT_RATE, SampleRate, StreamDirection};

pub(crate) type AudioPacket = Vec<u8, { spec::MAX_AUDIO_PACKET_BYTES }>;

// All accesses stay on core 0's thread-mode executor. ThreadModeMutex does not
// provide exclusion against RP2350 core 1 or interrupt-context access.
pub(crate) struct AudioQueue {
    packets: ThreadModeMutex<RefCell<Deque<AudioPacket, { spec::PACKET_QUEUE_CAPACITY }>>>,
}

impl AudioQueue {
    const fn new() -> Self {
        Self {
            packets: ThreadModeMutex::new(RefCell::new(Deque::new())),
        }
    }

    fn push(&self, packet: AudioPacket) {
        self.packets.lock(|packets| {
            let mut packets = packets.borrow_mut();
            if let Err(packet) = packets.push_back(packet) {
                assert!(packets.pop_front().is_some());
                assert!(packets.push_back(packet).is_ok());
            }
        });
    }

    fn pop(&self) -> Option<AudioPacket> {
        self.packets
            .lock(|packets| packets.borrow_mut().pop_front())
    }

    fn clear(&self) {
        self.packets.lock(|packets| packets.borrow_mut().clear());
    }
}

pub(crate) type AudioQueues = [AudioQueue; spec::FORMAT_RATE_COUNT];

pub(crate) type EndpointRateWatch = Watch<NoopRawMutex, EndpointRate, 1>;
pub(crate) type EndpointRateReceiver = WatchReceiver<'static, NoopRawMutex, EndpointRate, 1>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EndpointRate {
    Unset,
    Configured(SampleRate),
}

// Every physical endpoint owns an independent value and change generation.
// The watches are grouped only for lookup; they do not form a coherent duplex
// snapshot. NoopRawMutex is valid while all users remain on this executor.
pub(crate) struct EndpointRateWatches {
    playback: [EndpointRateWatch; spec::FORMAT_COUNT],
    capture: [EndpointRateWatch; spec::FORMAT_COUNT],
}

impl EndpointRateWatches {
    const fn new() -> Self {
        Self {
            playback: [const { EndpointRateWatch::new_with(EndpointRate::Unset) };
                spec::FORMAT_COUNT],
            capture: [const { EndpointRateWatch::new_with(EndpointRate::Unset) };
                spec::FORMAT_COUNT],
        }
    }

    pub(crate) fn current(&self, direction: StreamDirection, slot: usize) -> Option<EndpointRate> {
        self.watch(direction, slot)?.try_get()
    }

    pub(crate) fn set(&self, direction: StreamDirection, slot: usize, rate: EndpointRate) -> bool {
        let Some(watch) = self.watch(direction, slot) else {
            return false;
        };
        watch.sender().send(rate);
        true
    }

    pub(crate) fn notify(&self, direction: StreamDirection, slot: usize) -> bool {
        let Some(current) = self.current(direction, slot) else {
            return false;
        };
        self.set(direction, slot, current)
    }

    pub(crate) fn reset(&self) {
        for watch in &self.playback {
            watch.sender().send(EndpointRate::Unset);
        }
        for watch in &self.capture {
            watch.sender().send(EndpointRate::Unset);
        }
    }

    pub(crate) fn capture_receiver(&'static self, slot: usize) -> Option<EndpointRateReceiver> {
        self.capture.get(slot)?.receiver()
    }

    pub(crate) fn watch(
        &self,
        direction: StreamDirection,
        slot: usize,
    ) -> Option<&EndpointRateWatch> {
        match direction {
            StreamDirection::Playback => self.playback.get(slot),
            StreamDirection::Capture => self.capture.get(slot),
        }
    }
}

pub(crate) fn init_audio_queues() -> &'static AudioQueues {
    static QUEUES: StaticCell<AudioQueues> = StaticCell::new();
    QUEUES.init(core::array::from_fn(|_| AudioQueue::new()))
}

pub(crate) fn init_endpoint_rate_watches() -> &'static EndpointRateWatches {
    static WATCHES: StaticCell<EndpointRateWatches> = StaticCell::new();
    WATCHES.init(EndpointRateWatches::new())
}

pub(crate) async fn playback_task<'d, D: Driver<'d>>(
    slot: usize,
    mut endpoint: D::EndpointOut,
    rates: &'static EndpointRateWatches,
    queues: &'static AudioQueues,
) {
    let format =
        spec::format_by_endpoint_slot(slot).expect("playback task has a valid format slot");
    let max_packet_size = format.max_packet_size() as usize;

    // The destination rate is chosen only after the OUT read completes. This
    // also handles an endpoint read that remains pending across disable/re-enable.
    loop {
        endpoint.wait_enabled().await;

        let mut packet = AudioPacket::new();
        packet
            .resize(max_packet_size, 0)
            .expect("format MPS fits the audio packet");
        let len = match endpoint.read(packet.as_mut_slice()).await {
            Ok(len) => len,
            Err(EndpointError::Disabled) => continue,
            Err(EndpointError::BufferOverflow) => return,
        };

        if len == 0 || !len.is_multiple_of(format.audio_frame_bytes()) {
            continue;
        }
        let Some(EndpointRate::Configured(rate)) = rates.current(StreamDirection::Playback, slot)
        else {
            continue;
        };
        let Some(queue) = audio_queue(queues, slot, rate) else {
            continue;
        };

        packet.truncate(len);
        queue.push(packet);
    }
}

pub(crate) async fn capture_task<'d, D: Driver<'d>>(
    slot: usize,
    mut endpoint: D::EndpointIn,
    mut rate_updates: EndpointRateReceiver,
    queues: &'static AudioQueues,
) {
    let format = spec::format_by_endpoint_slot(slot).expect("capture task has a valid format slot");
    let mut current_rate = rate_updates.try_get().unwrap_or(EndpointRate::Unset);
    let mut silence = [0_u8; spec::MAX_AUDIO_PACKET_BYTES];
    let mut clock = PacketClock::new();

    apply_capture_rate(queues, slot, current_rate, &mut clock);

    // One endpoint operation is started per wait_enabled() pass. A control
    // change never cancels an operation already handed to the USB driver.
    loop {
        endpoint.wait_enabled().await;

        if let Some(updated) = rate_updates.try_changed() {
            current_rate = updated;
            apply_capture_rate(queues, slot, current_rate, &mut clock);
        }

        if let EndpointRate::Configured(rate) = current_rate {
            let queue = audio_queue(queues, slot, rate)
                .expect("configured endpoint rate has an audio queue");
            if let Some(packet) = queue.pop() {
                match endpoint.write(packet.as_slice()).await {
                    Ok(()) | Err(EndpointError::Disabled) => continue,
                    Err(EndpointError::BufferOverflow) => return,
                }
            }

            let len = clock.next_len(rate, format.audio_frame_bytes());
            silence[..len].fill(0);
            match endpoint.write(&silence[..len]).await {
                Ok(()) => diagnostics::record_in_loss(),
                Err(EndpointError::Disabled) => {}
                Err(EndpointError::BufferOverflow) => return,
            }
        } else {
            match endpoint.write(&[]).await {
                Ok(()) | Err(EndpointError::Disabled) => {}
                Err(EndpointError::BufferOverflow) => return,
            }
        }
    }
}

fn audio_queue(queues: &AudioQueues, format_slot: usize, rate: SampleRate) -> Option<&AudioQueue> {
    spec::audio_queue_index(format_slot, rate).and_then(|index| queues.get(index))
}

fn apply_capture_rate(
    queues: &AudioQueues,
    format_slot: usize,
    rate: EndpointRate,
    clock: &mut PacketClock,
) {
    clock.reset();
    if let EndpointRate::Configured(rate) = rate {
        audio_queue(queues, format_slot, rate)
            .expect("configured endpoint rate has an audio queue")
            .clear();
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

    fn reset(&mut self) {
        self.frame_bytes = 0;
        self.accumulator = 0;
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
    use super::{AudioPacket, AudioQueue, EndpointRate, EndpointRateWatches, PacketClock};
    use crate::spec::{self, SampleRate, StreamDirection, format_by_endpoint_slot};

    fn packet(value: u8) -> AudioPacket {
        let mut packet = AudioPacket::new();
        packet.push(value).unwrap();
        packet
    }

    #[test]
    fn full_queue_discards_only_the_oldest_packet() {
        let queue = AudioQueue::new();
        for value in 0..=spec::PACKET_QUEUE_CAPACITY {
            queue.push(packet(u8::try_from(value).unwrap()));
        }

        for value in 1..=spec::PACKET_QUEUE_CAPACITY {
            assert_eq!(
                queue.pop().unwrap().as_slice(),
                &[u8::try_from(value).unwrap()]
            );
        }
        assert!(queue.pop().is_none());
    }

    #[test]
    fn endpoint_rates_are_independent_and_same_value_notifies() {
        let rates = EndpointRateWatches::new();
        let mut capture = rates.capture[0].receiver().unwrap();
        assert_eq!(capture.try_get(), Some(EndpointRate::Unset));

        let configured = EndpointRate::Configured(SampleRate::R44100);
        assert!(rates.set(StreamDirection::Capture, 0, configured));
        assert_eq!(capture.try_changed(), Some(configured));
        assert_eq!(
            rates.current(StreamDirection::Playback, 0),
            Some(EndpointRate::Unset)
        );

        assert!(rates.notify(StreamDirection::Capture, 0));
        assert_eq!(capture.try_changed(), Some(configured));

        rates.reset();
        assert_eq!(capture.try_changed(), Some(EndpointRate::Unset));
        assert_eq!(
            rates.current(StreamDirection::Playback, 1),
            Some(EndpointRate::Unset)
        );
    }

    #[test]
    fn advertised_format_rates_have_unique_queues() {
        let mut seen = [false; spec::FORMAT_RATE_COUNT];
        for (format_slot, format) in spec::PCM_FORMATS.iter().enumerate() {
            for &rate in format.rates {
                let queue = spec::audio_queue_index(format_slot, rate).unwrap();
                assert!(!seen[queue]);
                seen[queue] = true;
            }
        }
        assert!(seen.into_iter().all(|value| value));
        assert_eq!(spec::audio_queue_index(2, SampleRate::R96000), None);
    }

    #[test]
    fn packet_clock_emits_fractional_cadence_and_resets() {
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
        clock.reset();
        assert_eq!(
            clock.next_len(SampleRate::R44100, frame_bytes),
            44 * frame_bytes
        );
    }
}
