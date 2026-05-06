use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};

use crate::audio::{AudioState, MAX_PACKET_SIZE, PIPE_SIZE, PacketClock, StreamDirection};

pub type AudioPipe = Pipe<CriticalSectionRawMutex, PIPE_SIZE>;

pub async fn out_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointOut,
    state: &'static AudioState,
    pipe: &'static AudioPipe,
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
                    let bytes_per_audio_frame = state.out_bytes_per_audio_frame();
                    if !state.loopback_format_matches()
                        || bytes_per_audio_frame == 0
                        || len % bytes_per_audio_frame != 0
                    {
                        pipe.clear();
                    } else if pipe.try_write(&packet[..len]).is_err() {
                        pipe.clear();
                        let _ = pipe.try_write(&packet[..len]);
                    }
                }
                Err(EndpointError::Disabled) => break,
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}

pub async fn in_task<'d, D: Driver<'d>>(
    mut ep: D::EndpointIn,
    state: &'static AudioState,
    pipe: &'static AudioPipe,
) {
    let mut packet = [0u8; MAX_PACKET_SIZE];
    let mut clock = PacketClock::new();

    loop {
        ep.wait_enabled().await;

        loop {
            let bytes_per_audio_frame = state.in_bytes_per_audio_frame();
            if bytes_per_audio_frame == 0 {
                embassy_futures::yield_now().await;
                continue;
            }

            let packet_len =
                clock.next_len(state.rate_hz(StreamDirection::In), bytes_per_audio_frame);
            packet[..packet_len].fill(0);

            if state.loopback_format_matches() {
                let mut offset = 0;
                while offset < packet_len {
                    match pipe.try_read(&mut packet[offset..packet_len]) {
                        Ok(0) => break,
                        Ok(read) => offset += read,
                        Err(_) => break,
                    }
                }
            } else {
                pipe.clear();
            }

            match ep.write(&packet[..packet_len]).await {
                Ok(()) => {}
                Err(EndpointError::Disabled) => break,
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}
