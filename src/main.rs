#![no_main]
#![no_std]
#![cfg_attr(test, allow(dead_code, unused_imports))]

mod audio;
mod control;
mod descriptors;
mod tasks;

#[cfg(test)]
mod tests;

#[cfg(not(test))]
use embassy_executor::Spawner;
#[cfg(not(test))]
use embassy_futures::join::join3;
use embassy_rp::{bind_interrupts, peripherals, usb};
#[cfg(not(test))]
use embassy_usb::{Builder, Config};
#[cfg(not(test))]
use panic_halt as _;
#[cfg(not(test))]
use static_cell::StaticCell;

use audio::AudioState;
#[cfg(not(test))]
use control::AudioControlHandler;
#[cfg(not(test))]
use descriptors::build_audio_function;
#[cfg(not(test))]
use tasks::{AudioPipe, in_task, out_task};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
});

static AUDIO_STATE: AudioState = AudioState::new();

#[cfg(not(test))]
static AUDIO_PIPE: AudioPipe = AudioPipe::new();

#[cfg(not(test))]
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = usb::Driver::new(p.USB, Irqs);

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 512]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 32]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 32]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HANDLER: StaticCell<AudioControlHandler> = StaticCell::new();

    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 512]);
    let bos_descriptor = BOS_DESCRIPTOR.init([0; 32]);
    let msos_descriptor = MSOS_DESCRIPTOR.init([0; 32]);
    let control_buf = CONTROL_BUF.init([0; 64]);

    let mut config = Config::new(0xcafe, 0x4001);
    config.manufacturer = Some("Embassy");
    config.product = Some("Pico 2 UAC1 Loopback");
    config.serial_number = Some("pico2-loopback-0001");
    config.max_packet_size_0 = 64;
    config.max_power = 100;

    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buf,
    );

    let endpoints = build_audio_function(&mut builder);
    let handler = HANDLER.init(AudioControlHandler::new(
        &AUDIO_STATE,
        &AUDIO_PIPE,
        endpoints.out_streaming_if,
        endpoints.in_streaming_if,
        endpoints.out_ep_addr,
        endpoints.in_ep_addr,
    ));
    builder.handler(handler);

    let mut usb = builder.build();

    join3(
        usb.run(),
        out_task::<usb::Driver<'static, peripherals::USB>>(
            endpoints.out_ep,
            &AUDIO_STATE,
            &AUDIO_PIPE,
        ),
        in_task::<usb::Driver<'static, peripherals::USB>>(
            endpoints.in_ep,
            &AUDIO_STATE,
            &AUDIO_PIPE,
        ),
    )
    .await;
}
