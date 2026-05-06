use embassy_rp::{Peri, bind_interrupts, peripherals, usb};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
});

pub type UsbDriver = usb::Driver<'static, peripherals::USB>;

pub fn usb_driver(usb_peripheral: Peri<'static, peripherals::USB>) -> UsbDriver {
    usb::Driver::new(usb_peripheral, Irqs)
}
