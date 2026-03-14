#![deny(unused_imports, unsafe_code, clippy::all)]
#![no_std]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

mod container;
mod map;
mod queue;

pub use self::queue::ZQueue;
