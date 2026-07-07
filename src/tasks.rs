use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use heapless::Vec;

use crate::audio::{self, AudioState, BitDepth, PACKET_QUEUE_SIZE, PacketClock};
use crate::diag::{self, InFallbackReason};

const MAX_PACKET_SIZE: usize = 97 * audio::bytes_per_audio_frame(BitDepth::Pcm24);

pub(crate) type AudioPacket = Vec<u8, MAX_PACKET_SIZE>;
pub(crate) type PacketQueue = Channel<CriticalSectionRawMutex, AudioPacket, PACKET_QUEUE_SIZE>;

pub(crate) async fn out_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointOut,
    state: &'static AudioState,
    packets: &'static PacketQueue,
) {
    let mut packet = [0u8; MAX_PACKET_SIZE];

    loop {
        ep.wait_enabled().await;

        loop {
            match ep.read(&mut packet).await {
                Ok(len) => {
                    if len == 0 {
                        continue;
                    }
                    let formats = state.formats();
                    let out_format = formats.out;
                    let bytes_per_audio_frame = out_format
                        .alternate_setting
                        .bit_depth()
                        .map_or(0, audio::bytes_per_audio_frame);
                    if !formats.loopback_format_matches()
                        || bytes_per_audio_frame == 0
                        || len % bytes_per_audio_frame != 0
                    {
                        packets.clear();
                    } else if enqueue_packet(packets, &packet[..len]).is_err() {
                        packets.clear();
                        let _ = enqueue_packet(packets, &packet[..len]);
                    }
                }
                Err(EndpointError::Disabled) => {
                    packets.clear();
                    break;
                }
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}

fn enqueue_packet(packets: &PacketQueue, packet: &[u8]) -> Result<(), ()> {
    let mut owned = AudioPacket::new();
    owned.extend_from_slice(packet).map_err(|_| ())?;
    packets.try_send(owned).map_err(|_| ())
}

pub(crate) async fn in_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointIn,
    state: &'static AudioState,
    packets: &'static PacketQueue,
) {
    let mut packet = [0u8; MAX_PACKET_SIZE];
    let mut clock = PacketClock::new();

    loop {
        ep.wait_enabled().await;

        loop {
            let formats = state.formats();
            let in_format = formats.in_;
            let Some(bit_depth) = in_format.alternate_setting.bit_depth() else {
                embassy_futures::yield_now().await;
                continue;
            };
            let bytes_per_audio_frame = audio::bytes_per_audio_frame(bit_depth);

            let format_matches = formats.loopback_format_matches();
            let mut fallback_reason = None;
            let loopback_packet = if format_matches {
                if let Ok(packet) = packets.try_receive() {
                    Some(packet)
                } else {
                    fallback_reason = Some(InFallbackReason::QueueEmpty);
                    None
                }
            } else {
                fallback_reason = Some(InFallbackReason::FormatMismatch);
                None
            };

            let write_result = if let Some(loopback_packet) = loopback_packet {
                ep.write(loopback_packet.as_slice()).await
            } else {
                let packet_len = clock.next_len(in_format.rate, bytes_per_audio_frame);
                packet[..packet_len].fill(0);
                if let Some(reason) = fallback_reason {
                    diag::add_in_fallback(reason, packet_len);
                }

                if !format_matches {
                    packets.clear();
                }

                ep.write(&packet[..packet_len]).await
            };

            match write_result {
                Err(EndpointError::Disabled) => {
                    packets.clear();
                    break;
                }
                Ok(()) | Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}
