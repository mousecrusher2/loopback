#![no_main]
#![no_std]

use embassy_executor::Spawner;
use embassy_usb::UsbDevice;
use embassy_usb::driver::Driver;
use embassy_usb::{Builder, Config};
use panic_halt as _;
use pico2_uac1_loopback::audio::AudioState;
use pico2_uac1_loopback::control::AudioControlHandler;
use pico2_uac1_loopback::descriptors::build_audio_function;
use pico2_uac1_loopback::irq::{UsbDriver, usb_driver};
use pico2_uac1_loopback::tasks::{AudioPipe, PacketLenQueue, in_task, out_task};
use static_cell::StaticCell;

static AUDIO_STATE: AudioState = AudioState::new();
static AUDIO_PIPE: AudioPipe = AudioPipe::new();
static PACKET_LENS: PacketLenQueue = PacketLenQueue::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = usb_driver(p.USB);

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 1024]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 32]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 32]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HANDLER: StaticCell<AudioControlHandler> = StaticCell::new();

    let config_descriptor = CONFIG_DESCRIPTOR.init([0; 1024]);
    let bos_descriptor = BOS_DESCRIPTOR.init([0; 32]);
    let msos_descriptor = MSOS_DESCRIPTOR.init([0; 32]);
    let control_buf = CONTROL_BUF.init([0; 64]);

    let mut config = Config::new(0xcafe, 0x4001);
    config.manufacturer = Some("Embassy");
    config.product = Some("Pico 2 UAC1 Loopback");
    config.serial_number = Some("pico2-loopback-0005");
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
        &PACKET_LENS,
        endpoints.out_streaming_if,
        endpoints.in_streaming_if,
        endpoints.out_ep_addrs,
        endpoints.in_ep_addrs,
    ));
    builder.handler(handler);

    spawner.spawn(usb_task(builder.build()).expect("USB task pool exhausted"));
    spawner.spawn(out_endpoint_task(endpoints.out_ep16).expect("OUT task pool exhausted"));
    spawner.spawn(out_endpoint_task(endpoints.out_ep24).expect("OUT task pool exhausted"));
    spawner.spawn(out_endpoint_task(endpoints.out_ep32).expect("OUT task pool exhausted"));
    spawner.spawn(in_endpoint_task(endpoints.in_ep16).expect("IN task pool exhausted"));
    spawner.spawn(in_endpoint_task(endpoints.in_ep24).expect("IN task pool exhausted"));
    spawner.spawn(in_endpoint_task(endpoints.in_ep32).expect("IN task pool exhausted"));
}

#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, UsbDriver>) {
    usb.run().await;
}

#[embassy_executor::task(pool_size = 3)]
async fn out_endpoint_task(ep: <UsbDriver as Driver<'static>>::EndpointOut) {
    out_task::<UsbDriver>(ep, &AUDIO_STATE, &AUDIO_PIPE, &PACKET_LENS).await;
}

#[embassy_executor::task(pool_size = 3)]
async fn in_endpoint_task(ep: <UsbDriver as Driver<'static>>::EndpointIn) {
    in_task::<UsbDriver>(ep, &AUDIO_STATE, &AUDIO_PIPE, &PACKET_LENS).await;
}
