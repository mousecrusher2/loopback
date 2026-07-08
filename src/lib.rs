#![no_std]
#![cfg_attr(test, no_main)]

use embassy_executor::Spawner;

mod app;
mod board;
mod control;
mod descriptors;
mod diagnostics;
mod spec;
mod streams;

pub fn run(spawner: Spawner) {
    app::run(spawner);
}
