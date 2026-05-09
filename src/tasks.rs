use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pipe::Pipe;
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};

use crate::audio::{AudioState, MAX_PACKET_SIZE, PACKET_LEN_QUEUE_SIZE, PIPE_SIZE, PacketClock};
use crate::diag::{self, InFallbackReason};

pub type AudioPipe = Pipe<CriticalSectionRawMutex, PIPE_SIZE>;
pub type PacketLenQueue = Channel<CriticalSectionRawMutex, u16, PACKET_LEN_QUEUE_SIZE>;

pub async fn out_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointOut,
    state: &'static AudioState,
    pipe: &'static AudioPipe,
    packet_lens: &'static PacketLenQueue,
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
                    diag::add_out_packet(len);
                    let formats = state.formats();
                    let out_format = formats.out;
                    let bytes_per_audio_frame = out_format.bytes_per_audio_frame();
                    if !formats.loopback_format_matches()
                        || bytes_per_audio_frame == 0
                        || len % bytes_per_audio_frame != 0
                    {
                        pipe.clear();
                        packet_lens.clear();
                    } else if enqueue_packet(pipe, packet_lens, &packet[..len]).is_err() {
                        pipe.clear();
                        packet_lens.clear();
                        diag::add_out_drop();
                        let _ = enqueue_packet(pipe, packet_lens, &packet[..len]);
                    }
                }
                Err(EndpointError::Disabled) => {
                    pipe.clear();
                    packet_lens.clear();
                    break;
                }
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}

fn enqueue_packet(pipe: &AudioPipe, packet_lens: &PacketLenQueue, packet: &[u8]) -> Result<(), ()> {
    if pipe.free_capacity() < packet.len() {
        return Err(());
    }

    let mut offset = 0;
    while offset < packet.len() {
        let written = pipe.try_write(&packet[offset..]).map_err(|_| ())?;
        if written == 0 {
            return Err(());
        }
        offset += written;
    }
    packet_lens.try_send(packet.len() as u16).map_err(|_| ())
}

pub async fn in_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointIn,
    state: &'static AudioState,
    pipe: &'static AudioPipe,
    packet_lens: &'static PacketLenQueue,
) {
    let mut packet = [0u8; MAX_PACKET_SIZE];
    let mut clock = PacketClock::new();

    loop {
        ep.wait_enabled().await;

        loop {
            let formats = state.formats();
            let in_format = formats.in_;
            let bytes_per_audio_frame = in_format.bytes_per_audio_frame();
            if bytes_per_audio_frame == 0 {
                embassy_futures::yield_now().await;
                continue;
            }

            let format_matches = formats.loopback_format_matches();
            let mut read_loopback = false;
            let mut fallback_reason = None;
            let packet_len = if format_matches {
                if let Ok(len) = packet_lens.try_receive() {
                    read_loopback = true;
                    usize::from(len)
                } else {
                    diag::add_in_queue_empty();
                    fallback_reason = Some(InFallbackReason::QueueEmpty);
                    clock.next_len(in_format.rate, bytes_per_audio_frame)
                }
            } else {
                fallback_reason = Some(InFallbackReason::FormatMismatch);
                clock.next_len(in_format.rate, bytes_per_audio_frame)
            };
            packet[..packet_len].fill(0);
            if let Some(reason) = fallback_reason {
                diag::add_in_fallback(reason, packet_len);
            }
            diag::add_in_packet(packet_len);

            if read_loopback {
                let mut offset = 0;
                while offset < packet_len {
                    match pipe.try_read(&mut packet[offset..packet_len]) {
                        Ok(0) => break,
                        Ok(read) => {
                            offset += read;
                            diag::add_in_pipe_bytes(read);
                        }
                        Err(_) => break,
                    }
                }
                if offset < packet_len {
                    let missing = packet_len - offset;
                    diag::add_in_underrun(missing);
                    diag::add_in_fallback(InFallbackReason::Underrun, missing);
                }
            } else if !format_matches {
                pipe.clear();
                packet_lens.clear();
            }

            match ep.write(&packet[..packet_len]).await {
                Ok(()) => {}
                Err(EndpointError::Disabled) => {
                    pipe.clear();
                    packet_lens.clear();
                    break;
                }
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}
