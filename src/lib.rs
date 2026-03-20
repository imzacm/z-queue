#![deny(unused_imports, unsafe_code, clippy::all)]
#![no_std]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

mod channel;
pub mod container;
#[cfg(feature = "map")]
mod map;
mod queue;

pub use self::channel::{Receiver, RecvError, SendError, Sender, bounded, unbounded};
#[cfg(feature = "map")]
pub use self::map::ZQueueMap;
pub use self::queue::ZQueue;
