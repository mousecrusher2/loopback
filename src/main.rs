#![no_main]
#![no_std]
#![cfg_attr(test, allow(dead_code, unused_imports))]

mod audio;
mod control;
mod descriptors;
mod irq;
#[cfg(not(test))]
mod runtime;
mod tasks;
