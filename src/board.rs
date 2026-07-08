use embassy_rp::{Peri, bind_interrupts, peripherals, usb};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
});

pub(crate) type UsbDriver = usb::Driver<'static, peripherals::USB>;

pub(crate) fn usb_driver(usb: Peri<'static, peripherals::USB>) -> UsbDriver {
    usb::Driver::new(usb, Irqs)
}
