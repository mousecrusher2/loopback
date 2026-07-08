use embassy_executor::Spawner;
use embassy_usb as usb;
use embassy_usb::driver::Driver;
use static_cell::StaticCell;

use crate::board::{self, UsbDriver};
use crate::control::AudioControl;
use crate::descriptors::{AudioEndpoints, build_audio_function};
use crate::diagnostics::fallback_led_task;
use crate::streams::{AudioQueue, StreamState, capture_task, playback_task};

static STREAM_STATE: StreamState = StreamState::new();
static AUDIO_QUEUE: AudioQueue = AudioQueue::new();

pub(crate) fn run(spawner: Spawner) {
    let peripherals = embassy_rp::init(embassy_rp::config::Config::default());
    let driver = board::usb_driver(peripherals.USB);

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

    let mut builder = usb::Builder::new(
        driver,
        usb_config(),
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buf,
    );

    let endpoints = build_audio_function(&mut builder);
    let handler = {
        static CELL: StaticCell<AudioControl> = StaticCell::new();
        CELL.init(AudioControl::new(
            &STREAM_STATE,
            &AUDIO_QUEUE,
            endpoints.routing,
        ))
    };
    builder.handler(handler);

    spawn_usb(spawner, builder.build(), endpoints);
    spawner.spawn(fallback_led_task(peripherals.PIN_25).unwrap());
}

fn usb_config() -> usb::Config<'static> {
    let mut config = usb::Config::new(crate::spec::USB_VENDOR_ID, crate::spec::USB_PRODUCT_ID);
    config.manufacturer = Some(crate::spec::USB_MANUFACTURER);
    config.product = Some(crate::spec::USB_PRODUCT);
    config.serial_number = Some(crate::spec::USB_SERIAL);
    config.max_packet_size_0 = crate::spec::USB_MAX_PACKET_SIZE_0;
    config.max_power = crate::spec::USB_MAX_POWER_MA;
    config
}

fn spawn_usb(
    spawner: Spawner,
    usb_device: usb::UsbDevice<'static, UsbDriver>,
    endpoints: AudioEndpoints<'static, UsbDriver>,
) {
    spawner.spawn(usb_task(usb_device).unwrap());
    spawner.spawn(playback_endpoint_task(endpoints.playback_16).unwrap());
    spawner.spawn(playback_endpoint_task(endpoints.playback_24).unwrap());
    spawner.spawn(playback_endpoint_task(endpoints.playback_32).unwrap());
    spawner.spawn(capture_endpoint_task(endpoints.capture_16).unwrap());
    spawner.spawn(capture_endpoint_task(endpoints.capture_24).unwrap());
    spawner.spawn(capture_endpoint_task(endpoints.capture_32).unwrap());
}

#[embassy_executor::task]
async fn usb_task(mut usb_device: usb::UsbDevice<'static, UsbDriver>) {
    usb_device.run().await;
}

#[embassy_executor::task(pool_size = 3)]
async fn playback_endpoint_task(endpoint: <UsbDriver as Driver<'static>>::EndpointOut) {
    playback_task::<UsbDriver>(endpoint, &STREAM_STATE, &AUDIO_QUEUE).await;
}

#[embassy_executor::task(pool_size = 3)]
async fn capture_endpoint_task(endpoint: <UsbDriver as Driver<'static>>::EndpointIn) {
    capture_task::<UsbDriver>(endpoint, &STREAM_STATE, &AUDIO_QUEUE).await;
}
