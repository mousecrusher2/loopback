use core::sync::atomic::{AtomicU32, Ordering};

use embassy_rp::Peri;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::PIN_25;
use embassy_time::{Duration, Timer};

static IN_SILENCE_PACKETS: AtomicU32 = AtomicU32::new(0);

pub(crate) fn record_in_silence() {
    IN_SILENCE_PACKETS.fetch_add(1, Ordering::Relaxed);
}

fn in_silence_packets() -> u32 {
    IN_SILENCE_PACKETS.load(Ordering::Relaxed)
}

struct LedHold {
    last_seen: u32,
    remaining_ticks: u16,
    hold_ticks: u16,
}

impl LedHold {
    const fn new(initial_count: u32, hold_ticks: u16) -> Self {
        Self {
            last_seen: initial_count,
            remaining_ticks: 0,
            hold_ticks,
        }
    }

    fn step(&mut self, current_count: u32) -> bool {
        if current_count != self.last_seen {
            self.last_seen = current_count;
            self.remaining_ticks = self.hold_ticks;
            return true;
        }

        if self.remaining_ticks == 0 {
            return false;
        }

        self.remaining_ticks -= 1;
        self.remaining_ticks != 0
    }
}

#[embassy_executor::task]
pub(crate) async fn fallback_led_task(pin: Peri<'static, PIN_25>) {
    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    const HOLD_TICKS: u16 = 100;

    let mut led = Output::new(pin, Level::Low);
    let mut hold = LedHold::new(in_silence_packets(), HOLD_TICKS);

    loop {
        if hold.step(in_silence_packets()) {
            led.set_high();
        } else {
            led.set_low();
        }

        Timer::after(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::LedHold;

    #[test]
    fn led_hold_extends_from_latest_activity() {
        let mut hold = LedHold::new(0, 3);

        assert!(!hold.step(0));
        assert!(hold.step(1));
        assert!(hold.step(1));

        assert!(hold.step(2));
        assert!(hold.step(2));
        assert!(hold.step(2));
        assert!(!hold.step(2));
    }
}
