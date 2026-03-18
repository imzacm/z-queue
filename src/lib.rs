#![deny(unused_imports, unsafe_code, clippy::all)]
#![no_std]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

pub mod container;
#[cfg(feature = "map")]
mod map;
mod queue;

#[cfg(feature = "map")]
pub use self::map::ZQueueMap;
pub use self::queue::ZQueue;
