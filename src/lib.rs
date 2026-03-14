#![deny(unused_imports, unsafe_code, clippy::all)]
#![no_std]

extern crate alloc;

mod container;
mod queue;

pub use self::queue::ZQueue;
