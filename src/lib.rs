#![deny(unused_imports, clippy::all)]
#![no_std]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

mod channel;
pub mod container;
pub mod defaults;
#[cfg(feature = "map")]
mod map;
pub mod notify;
mod queue;

#[cfg(feature = "stream")]
pub use self::channel::RecvStream;
pub use self::channel::{Receiver, RecvError, SendError, Sender, bounded, unbounded};
#[cfg(feature = "map")]
pub use self::map::ZQueueMap;
pub use self::queue::ZQueue;
