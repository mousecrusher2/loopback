#![no_std]
#![cfg_attr(test, no_main)]

use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::{Peri, peripherals};
use embassy_time::Timer;
use embassy_usb as usb;
use static_cell::StaticCell;
use usb::driver::Driver;

pub mod audio;
pub mod control;
pub mod descriptors;
pub mod diag;
pub mod irq;
pub mod tasks;

use audio::AudioState;
use control::AudioControlHandler;
use descriptors::build_audio_function;
use irq::{UsbDriver, usb_driver};
use tasks::{PacketQueue, in_task, out_task};

static AUDIO_STATE: AudioState = AudioState::new();
static PACKETS: PacketQueue = PacketQueue::new();

#[allow(clippy::missing_panics_doc)]
pub fn lib_main(spawner: Spawner) {
    let p = embassy_rp::init(embassy_rp::config::Config::default());
    let driver = usb_driver(p.USB);

    let config_descriptor = {
        static CELL: StaticCell<[u8; 1024]> = StaticCell::new();
        CELL.init([0; 1024])
    };
    let bos_descriptor = {
        static CELL: StaticCell<[u8; 32]> = StaticCell::new();
        CELL.init([0; 32])
    };
    let msos_descriptor = {
        static CELL: StaticCell<[u8; 32]> = StaticCell::new();
        CELL.init([0; 32])
    };
    let control_buf = {
        static CELL: StaticCell<[u8; 64]> = StaticCell::new();
        CELL.init([0; 64])
    };

    let config = {
        let mut config = usb::Config::new(0xcafe, 0x4001);
        config.manufacturer = Some("Embassy");
        config.product = Some("Pico 2 UAC1 Loopback");
        config.serial_number = Some("pico2-loopback-0005");
        config.max_packet_size_0 = 64;
        config.max_power = 100;
        config
    };

    let mut builder = usb::Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buf,
    );

    let endpoints = build_audio_function(&mut builder);
    let handler = {
        static CELL: StaticCell<AudioControlHandler> = StaticCell::new();
        CELL.init(AudioControlHandler::new(
            &AUDIO_STATE,
            &PACKETS,
            endpoints.out_streaming_if,
            endpoints.in_streaming_if,
            endpoints.out_ep_addrs,
            endpoints.in_ep_addrs,
        ))
    };
    builder.handler(handler);

    spawner.spawn(usb_task(builder.build()).unwrap());
    spawner.spawn(out_endpoint_task(endpoints.out_ep16).unwrap());
    spawner.spawn(out_endpoint_task(endpoints.out_ep24).unwrap());
    spawner.spawn(out_endpoint_task(endpoints.out_ep32).unwrap());
    spawner.spawn(in_endpoint_task(endpoints.in_ep16).unwrap());
    spawner.spawn(in_endpoint_task(endpoints.in_ep24).unwrap());
    spawner.spawn(in_endpoint_task(endpoints.in_ep32).unwrap());
    spawner.spawn(fallback_led_task(p.PIN_25).unwrap());
}

#[embassy_executor::task]
async fn usb_task(mut usb_device: usb::UsbDevice<'static, UsbDriver>) {
    usb_device.run().await;
}

#[embassy_executor::task(pool_size = 3)]
async fn out_endpoint_task(ep: <UsbDriver as Driver<'static>>::EndpointOut) {
    out_task::<UsbDriver>(ep, &AUDIO_STATE, &PACKETS).await;
}

#[embassy_executor::task(pool_size = 3)]
async fn in_endpoint_task(ep: <UsbDriver as Driver<'static>>::EndpointIn) {
    in_task::<UsbDriver>(ep, &AUDIO_STATE, &PACKETS).await;
}

#[embassy_executor::task]
async fn fallback_led_task(pin: Peri<'static, peripherals::PIN_25>) {
    const POLL_MS: u64 = 10;
    const HOLD_TICKS: u64 = 1_000 / POLL_MS;

    let mut led = Output::new(pin, Level::Low);
    let mut last_seen = diag::in_fallback_packets();
    let mut hold_ticks = 0;

    loop {
        let current = diag::in_fallback_packets();
        if current != last_seen {
            last_seen = current;
            hold_ticks = HOLD_TICKS;
            led.set_high();
        } else if hold_ticks > 0 {
            hold_ticks -= 1;
            if hold_ticks == 0 {
                led.set_low();
            }
        }

        Timer::after_millis(POLL_MS).await;
    }
}
