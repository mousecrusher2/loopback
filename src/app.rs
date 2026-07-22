use embassy_executor::Spawner;
use embassy_usb as usb;
use embassy_usb::driver::Driver;
use static_cell::StaticCell;

use crate::board::{self, UsbDriver};
use crate::control::AudioControl;
use crate::descriptors::{AudioEndpoints, build_audio_function};
use crate::diagnostics::loss_led_task;
use crate::spec::{PcmFormat, StreamDirection};
use crate::streams::{
    AudioQueue, AudioQueues, EndpointRateReceiver, EndpointRateWatch, EndpointRateWatches,
    capture_task, init_audio_queues, init_endpoint_rate_watches, playback_task,
};

pub(crate) fn run(spawner: Spawner) {
    let peripherals = embassy_rp::init(embassy_rp::config::Config::default());
    let driver = board::usb_driver(peripherals.USB);
    let rates = init_endpoint_rate_watches();
    let queues = init_audio_queues();

    let config_descriptor = {
        static CELL: StaticCell<[u8; crate::spec::CONFIG_DESCRIPTOR_CAPACITY]> = StaticCell::new();
        CELL.init([0; crate::spec::CONFIG_DESCRIPTOR_CAPACITY])
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
    let control_map = endpoints.control_map();
    let handler = {
        static CELL: StaticCell<AudioControl> = StaticCell::new();
        CELL.init(AudioControl::new(rates, control_map))
    };
    builder.handler(handler);

    spawn_usb(spawner, builder.build(), endpoints, rates, queues);
    spawner.spawn(loss_led_task(peripherals.PIN_25).unwrap());
}

fn usb_config() -> usb::Config<'static> {
    let mut config = usb::Config::new(crate::spec::USB_VENDOR_ID, crate::spec::USB_PRODUCT_ID);
    config.manufacturer = Some(crate::spec::USB_MANUFACTURER);
    config.product = Some(crate::spec::USB_PRODUCT);
    config.max_packet_size_0 = crate::spec::USB_MAX_PACKET_SIZE_0;
    config.max_power = crate::spec::USB_MAX_POWER_MA;
    config
}

fn spawn_usb(
    spawner: Spawner,
    usb_device: usb::UsbDevice<'static, UsbDriver>,
    endpoints: AudioEndpoints<'static, UsbDriver>,
    rates: &'static EndpointRateWatches,
    queues: &'static AudioQueues,
) {
    let playback = endpoints.playback;
    let capture = endpoints.capture;
    let mut remaining_queues: &'static [AudioQueue] = queues;

    spawner.spawn(usb_task(usb_device).unwrap());
    // Each endpoint keeps a persistent task. This avoids dynamically dispatching
    // or cancelling endpoint futures when the host changes alternate settings.
    for (slot, ((playback, capture), format)) in playback
        .into_iter()
        .zip(capture)
        .zip(crate::spec::PCM_FORMATS)
        .enumerate()
    {
        let format_queues = remaining_queues
            .split_off(..format.rates.len())
            .expect("one queue exists for each advertised format rate");
        let rate_watch = rates
            .watch(StreamDirection::Playback, slot)
            .expect("one rate Watch exists for each playback endpoint");
        let rate_updates = rates
            .capture_receiver(slot)
            .expect("one rate receiver exists for each capture endpoint");
        spawner.spawn(playback_endpoint_task(playback, format, rate_watch, format_queues).unwrap());
        spawner.spawn(capture_endpoint_task(capture, format, rate_updates, format_queues).unwrap());
    }
    assert!(remaining_queues.is_empty());
}

#[embassy_executor::task]
async fn usb_task(mut usb_device: usb::UsbDevice<'static, UsbDriver>) {
    usb_device.run().await;
}

#[embassy_executor::task(pool_size = crate::spec::FORMAT_COUNT)]
async fn playback_endpoint_task(
    endpoint: <UsbDriver as Driver<'static>>::EndpointOut,
    format: &'static PcmFormat,
    rate_watch: &'static EndpointRateWatch,
    queues: &'static [AudioQueue],
) {
    playback_task::<UsbDriver>(endpoint, format, rate_watch, queues).await;
}

#[embassy_executor::task(pool_size = crate::spec::FORMAT_COUNT)]
async fn capture_endpoint_task(
    endpoint: <UsbDriver as Driver<'static>>::EndpointIn,
    format: &'static PcmFormat,
    rate_updates: EndpointRateReceiver,
    queues: &'static [AudioQueue],
) {
    capture_task::<UsbDriver>(endpoint, format, rate_updates, queues).await;
}
