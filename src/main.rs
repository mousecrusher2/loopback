#![no_main]
#![no_std]

use embassy_executor::Spawner;
use panic_halt as _;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    pico2_uac1_loopback::run(spawner);
}
