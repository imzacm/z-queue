use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread;

use criterion::{Criterion, criterion_group, criterion_main};
use tokio::runtime::Runtime;
use z_queue::ZQueue;
#[cfg(feature = "segmented-array")]
use z_queue::container::SegmentedArray;
#[cfg(feature = "crossbeam-queue")]
use z_queue::container::{CrossbeamArrayQueue, CrossbeamSegQueue};
use z_queue::container::{Swap, VecDeque};

const MESSAGES: usize = 1000;
const BOUND: usize = 100;
const BOUND_NON_ZERO: NonZeroUsize = match NonZeroUsize::new(BOUND) {
    Some(v) => v,
    None => unreachable!(),
};

const PRODUCERS: usize = 4;
const CONSUMERS: usize = 4;

// Helper to easily clone Arc-wrapped ZQueues
fn zqueue_pair<Q>(queue: ZQueue<Q>) -> (Arc<ZQueue<Q>>, Arc<ZQueue<Q>>) {
    let arc = Arc::new(queue);
    (arc.clone(), arc)
}

// ==========================================
// --- SYNC MACROS ---
// ==========================================

macro_rules! bench_sync_spsc {
    ($group:expr, $name:expr, $setup:expr, |$tx_base:ident| $clone_tx:expr, ($tx:ident, $val:ident) => $send:expr, |$rx:ident| $recv:expr) => {
        $group.bench_function($name, |b| {
            b.iter(|| {
                #[allow(unused_mut)]
                let (tx_base, mut rx_base) = $setup;
                let $tx_base = &tx_base;
                let $tx = $clone_tx;
                let handle = thread::spawn(move || {
                    for $val in 0..MESSAGES {
                        $send;
                    }
                });
                let $rx = &mut rx_base;
                for _ in 0..MESSAGES {
                    std::hint::black_box($recv);
                }

                handle.join().unwrap();
            })
        });
    };
}

macro_rules! bench_sync_mpsc {
    ($group:expr, $name:expr, $setup:expr, |$tx_base:ident| $clone_tx:expr, ($tx:ident, $val:ident) => $send:expr, |$rx:ident| $recv:expr) => {
        $group.bench_function($name, |b| {
            b.iter(|| {
                #[allow(unused_mut)]
                let (tx_base, mut rx_base) = $setup;
                let msgs_per_prod = MESSAGES / PRODUCERS;
                let mut p_handles = Vec::with_capacity(PRODUCERS);
                for _ in 0..PRODUCERS {
                    let $tx_base = &tx_base;
                    let $tx = $clone_tx;
                    p_handles.push(thread::spawn(move || {
                        for $val in 0..msgs_per_prod {
                            $send;
                        }
                    }));
                }
                let $rx = &mut rx_base;
                for _ in 0..MESSAGES {
                    std::hint::black_box($recv);
                }
                for h in p_handles {
                    h.join().unwrap();
                }
            })
        });
    };
}

macro_rules! bench_sync_mpmc {
    ($group:expr, $name:expr, $setup:expr, |$tx_base:ident| $clone_tx:expr, |$rx_base:ident| $clone_rx:expr, ($tx:ident, $val:ident) => $send:expr, |$rx:ident| $recv:expr) => {
        $group.bench_function($name, |b| {
            b.iter(|| {
                let (tx_base, rx_base) = $setup;
                let msgs_per_prod = MESSAGES / PRODUCERS;
                let msgs_per_cons = MESSAGES / CONSUMERS;

                let mut p_handles = Vec::with_capacity(PRODUCERS);
                for _ in 0..PRODUCERS {
                    let $tx_base = &tx_base;
                    let $tx = $clone_tx;
                    p_handles.push(thread::spawn(move || {
                        for $val in 0..msgs_per_prod {
                            $send;
                        }
                    }));
                }

                let mut c_handles = Vec::with_capacity(CONSUMERS);
                for _ in 0..CONSUMERS {
                    let $rx_base = &rx_base;
                    let $rx = $clone_rx;
                    c_handles.push(thread::spawn(move || {
                        for _ in 0..msgs_per_cons {
                            std::hint::black_box($recv);
                        }
                    }));
                }

                for h in c_handles {
                    h.join().unwrap();
                }
                for h in p_handles {
                    h.join().unwrap();
                }
            })
        });
    };
}

// Z-Queue Sync Helpers
macro_rules! bench_zq_sync_spsc { ($g:expr, $name:expr, $init:expr) => { bench_sync_spsc!($g, $name, zqueue_pair($init), |tx_base| tx_base.clone(), (tx, i) => tx.push(i), |rx| rx.pop()); }; }
macro_rules! bench_zq_sync_mpsc { ($g:expr, $name:expr, $init:expr) => { bench_sync_mpsc!($g, $name, zqueue_pair($init), |tx_base| tx_base.clone(), (tx, i) => tx.push(i), |rx| rx.pop()); }; }
macro_rules! bench_zq_sync_mpmc { ($g:expr, $name:expr, $init:expr) => { bench_sync_mpmc!($g, $name, zqueue_pair($init), |tx_base| tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.push(i), |rx| rx.pop()); }; }

// ==========================================
// --- ASYNC MACROS ---
// ==========================================

macro_rules! bench_async_spsc {
    ($group:expr, $name:expr, $rt:expr, $setup:expr, |$tx_base:ident| $clone_tx:expr, ($tx:ident, $val:ident) => $send:expr, |$rx:ident| $recv:expr) => {
        $group.bench_function($name, |b| {
            b.to_async($rt).iter(|| async {
                #[allow(unused_mut)]
                let (tx_base, mut rx_base) = $setup;
                let $tx_base = &tx_base;
                let $tx = $clone_tx;
                let handle = tokio::spawn(async move {
                    for $val in 0..MESSAGES {
                        $send;
                    }
                });
                let $rx = &mut rx_base;
                for _ in 0..MESSAGES {
                    std::hint::black_box($recv);
                }
                handle.await.unwrap();
            })
        });
    };
}

macro_rules! bench_async_mpsc {
    ($group:expr, $name:expr, $rt:expr, $setup:expr, |$tx_base:ident| $clone_tx:expr, ($tx:ident, $val:ident) => $send:expr, |$rx:ident| $recv:expr) => {
        $group.bench_function($name, |b| {
            b.to_async($rt).iter(|| async {
                #[allow(unused_mut)]
                let (tx_base, mut rx_base) = $setup;
                let msgs_per_prod = MESSAGES / PRODUCERS;
                let mut p_handles = Vec::with_capacity(PRODUCERS);
                for _ in 0..PRODUCERS {
                    let $tx_base = &tx_base;
                    let $tx = $clone_tx;
                    p_handles.push(tokio::spawn(async move {
                        for $val in 0..msgs_per_prod {
                            $send;
                        }
                    }));
                }
                let $rx = &mut rx_base;
                for _ in 0..MESSAGES {
                    std::hint::black_box($recv);
                }
                for h in p_handles {
                    h.await.unwrap();
                }
            })
        });
    };
}

macro_rules! bench_async_mpmc {
    ($group:expr, $name:expr, $rt:expr, $setup:expr, |$tx_base:ident| $clone_tx:expr, |$rx_base:ident| $clone_rx:expr, ($tx:ident, $val:ident) => $send:expr, |$rx:ident| $recv:expr) => {
        $group.bench_function($name, |b| {
            b.to_async($rt).iter(|| async {
                let (tx_base, rx_base) = $setup;
                let msgs_per_prod = MESSAGES / PRODUCERS;
                let msgs_per_cons = MESSAGES / CONSUMERS;

                let mut p_handles = Vec::with_capacity(PRODUCERS);
                for _ in 0..PRODUCERS {
                    let $tx_base = &tx_base;
                    let $tx = $clone_tx;
                    p_handles.push(tokio::spawn(async move {
                        for $val in 0..msgs_per_prod {
                            $send;
                        }
                    }));
                }

                let mut c_handles = Vec::with_capacity(CONSUMERS);
                for _ in 0..CONSUMERS {
                    let $rx_base = &rx_base;
                    let $rx = $clone_rx;
                    c_handles.push(tokio::spawn(async move {
                        for _ in 0..msgs_per_cons {
                            std::hint::black_box($recv);
                        }
                    }));
                }

                for h in c_handles {
                    h.await.unwrap();
                }
                for h in p_handles {
                    h.await.unwrap();
                }
            })
        });
    };
}

// Z-Queue Async Helpers
macro_rules! bench_zq_async_spsc { ($g:expr, $name:expr, $rt:expr, $init:expr) => { bench_async_spsc!($g, $name, $rt, zqueue_pair($init), |tx_base| tx_base.clone(), (tx, i) => tx.push_async(i).await, |rx| rx.pop_async().await); }; }
macro_rules! bench_zq_async_mpsc { ($g:expr, $name:expr, $rt:expr, $init:expr) => { bench_async_mpsc!($g, $name, $rt, zqueue_pair($init), |tx_base| tx_base.clone(), (tx, i) => tx.push_async(i).await, |rx| rx.pop_async().await); }; }
macro_rules! bench_zq_async_mpmc { ($g:expr, $name:expr, $rt:expr, $init:expr) => { bench_async_mpmc!($g, $name, $rt, zqueue_pair($init), |tx_base| tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.push_async(i).await, |rx| rx.pop_async().await); }; }

// ==========================================
// 1. UNBOUNDED SYNC BENCHMARKS
// ==========================================
fn bench_unbounded_sync(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("Unbounded Sync SPSC");
        bench_sync_spsc!(&mut group, "std::sync::mpsc", std::sync::mpsc::channel(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_spsc!(&mut group, "crossbeam_channel", crossbeam_channel::unbounded(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_spsc!(&mut group, "flume", flume::unbounded(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue Crossbeam SegQueue",
            ZQueue::<CrossbeamSegQueue<_>>::unbounded()
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            ZQueue::<SegmentedArray<_, 64>>::unbounded()
        );
        bench_zq_sync_spsc!(&mut group, "z-queue VecDeque", ZQueue::<VecDeque<_>>::unbounded());
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            ZQueue::<Swap<VecDeque<_>, 2>>::unbounded()
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Unbounded Sync MPSC");
        bench_sync_mpsc!(&mut group, "std::sync::mpsc", std::sync::mpsc::channel(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_mpsc!(&mut group, "crossbeam_channel", crossbeam_channel::unbounded(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_mpsc!(&mut group, "flume", flume::unbounded(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue Crossbeam SegQueue",
            ZQueue::<CrossbeamSegQueue<_>>::unbounded()
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            ZQueue::<SegmentedArray<_, 64>>::unbounded()
        );
        bench_zq_sync_mpsc!(&mut group, "z-queue VecDeque", ZQueue::<VecDeque<_>>::unbounded());
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            ZQueue::<Swap<VecDeque<_>, 2>>::unbounded()
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Unbounded Sync MPMC");
        // std::sync::mpsc omitted (SPSC/MPSC only)
        bench_sync_mpmc!(&mut group, "crossbeam_channel", crossbeam_channel::unbounded(), |tx_base| tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_mpmc!(&mut group, "flume", flume::unbounded(), |tx_base| tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue Crossbeam SegQueue",
            ZQueue::<CrossbeamSegQueue<_>>::unbounded()
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue SegmentedArray 64",
            ZQueue::<SegmentedArray<_, 64>>::unbounded()
        );
        bench_zq_sync_mpmc!(&mut group, "z-queue VecDeque", ZQueue::<VecDeque<_>>::unbounded());
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            ZQueue::<Swap<VecDeque<_>, 2>>::unbounded()
        );
        group.finish();
    }
}

// ==========================================
// 2. BOUNDED SYNC BENCHMARKS
// ==========================================
fn bench_bounded_sync(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("Bounded Sync SPSC");
        bench_sync_spsc!(&mut group, "std::sync::mpsc::sync_channel", std::sync::mpsc::sync_channel(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_spsc!(&mut group, "crossbeam_channel", crossbeam_channel::bounded(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_spsc!(&mut group, "flume", flume::bounded(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue Crossbeam ArrayQueue",
            ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO)
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue VecDeque",
            ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_sync_spsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO)
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Bounded Sync MPSC");
        bench_sync_mpsc!(&mut group, "std::sync::mpsc::sync_channel", std::sync::mpsc::sync_channel(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_mpsc!(&mut group, "crossbeam_channel", crossbeam_channel::bounded(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_mpsc!(&mut group, "flume", flume::bounded(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue Crossbeam ArrayQueue",
            ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO)
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue VecDeque",
            ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_sync_mpsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO)
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Bounded Sync MPMC");
        bench_sync_mpmc!(&mut group, "crossbeam_channel", crossbeam_channel::bounded(BOUND), |tx_base| tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        bench_sync_mpmc!(&mut group, "flume", flume::bounded(BOUND), |tx_base| tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue Crossbeam ArrayQueue",
            ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO)
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue SegmentedArray 64",
            ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue VecDeque",
            ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_sync_mpmc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO)
        );
        group.finish();
    }
}

// ==========================================
// 3. UNBOUNDED ASYNC BENCHMARKS
// ==========================================
fn bench_unbounded_async(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    {
        let mut group = c.benchmark_group("Unbounded Async SPSC");
        bench_async_spsc!(&mut group, "tokio::sync::mpsc", &rt, tokio::sync::mpsc::unbounded_channel(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().await.unwrap());
        bench_async_spsc!(&mut group, "flume", &rt, flume::unbounded(), |tx_base| tx_base.clone(), (tx, i) => tx.send_async(i).await.unwrap(), |rx| rx.recv_async().await.unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_async_spsc!(
            &mut group,
            "z-queue Crossbeam SegQueue",
            &rt,
            ZQueue::<CrossbeamSegQueue<_>>::unbounded()
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_async_spsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            &rt,
            ZQueue::<SegmentedArray<_, 64>>::unbounded()
        );
        bench_zq_async_spsc!(
            &mut group,
            "z-queue VecDeque",
            &rt,
            ZQueue::<VecDeque<_>>::unbounded()
        );
        bench_zq_async_spsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            &rt,
            ZQueue::<Swap<VecDeque<_>, 2>>::unbounded()
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Unbounded Async MPSC");
        bench_async_mpsc!(&mut group, "tokio::sync::mpsc", &rt, tokio::sync::mpsc::unbounded_channel(), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).unwrap(), |rx| rx.recv().await.unwrap());
        bench_async_mpsc!(&mut group, "flume", &rt, flume::unbounded(), |tx_base| tx_base.clone(), (tx, i) => tx.send_async(i).await.unwrap(), |rx| rx.recv_async().await.unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue Crossbeam SegQueue",
            &rt,
            ZQueue::<CrossbeamSegQueue<_>>::unbounded()
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            &rt,
            ZQueue::<SegmentedArray<_, 64>>::unbounded()
        );
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue VecDeque",
            &rt,
            ZQueue::<VecDeque<_>>::unbounded()
        );
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            &rt,
            ZQueue::<Swap<VecDeque<_>, 2>>::unbounded()
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Unbounded Async MPMC");
        // Flume never gets past the warmup.
        // bench_async_mpmc!(&mut group, "flume", &rt, flume::unbounded(), |tx_base|
        // tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.send_async(i).await.unwrap(),
        // |rx| rx.recv_async().await.unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue Crossbeam SegQueue",
            &rt,
            ZQueue::<CrossbeamSegQueue<_>>::unbounded()
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue SegmentedArray 64",
            &rt,
            ZQueue::<SegmentedArray<_, 64>>::unbounded()
        );
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue VecDeque",
            &rt,
            ZQueue::<VecDeque<_>>::unbounded()
        );
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            &rt,
            ZQueue::<Swap<VecDeque<_>, 2>>::unbounded()
        );
        group.finish();
    }
}

// ==========================================
// 4. BOUNDED ASYNC BENCHMARKS
// ==========================================
fn bench_bounded_async(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    {
        let mut group = c.benchmark_group("Bounded Async SPSC");
        bench_async_spsc!(&mut group, "tokio::sync::mpsc", &rt, tokio::sync::mpsc::channel(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).await.unwrap(), |rx| rx.recv().await.unwrap());
        bench_async_spsc!(&mut group, "flume", &rt, flume::bounded(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send_async(i).await.unwrap(), |rx| rx.recv_async().await.unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_async_spsc!(
            &mut group,
            "z-queue Crossbeam ArrayQueue",
            &rt,
            ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO)
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_async_spsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            &rt,
            ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_async_spsc!(
            &mut group,
            "z-queue VecDeque",
            &rt,
            ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_async_spsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            &rt,
            ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO)
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Bounded Async MPSC");
        bench_async_mpsc!(&mut group, "tokio::sync::mpsc", &rt, tokio::sync::mpsc::channel(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send(i).await.unwrap(), |rx| rx.recv().await.unwrap());
        bench_async_mpsc!(&mut group, "flume", &rt, flume::bounded(BOUND), |tx_base| tx_base.clone(), (tx, i) => tx.send_async(i).await.unwrap(), |rx| rx.recv_async().await.unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue Crossbeam ArrayQueue",
            &rt,
            ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO)
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue SegmentedArray 64",
            &rt,
            ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue VecDeque",
            &rt,
            ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_async_mpsc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            &rt,
            ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO)
        );
        group.finish();
    }
    {
        let mut group = c.benchmark_group("Bounded Async MPMC");
        // Flume never gets past the warmup.
        // bench_async_mpmc!(&mut group, "flume", &rt, flume::bounded(BOUND), |tx_base|
        // tx_base.clone(), |rx_base| rx_base.clone(), (tx, i) => tx.send_async(i).await.unwrap(),
        // |rx| rx.recv_async().await.unwrap());
        #[cfg(feature = "crossbeam-queue")]
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue Crossbeam ArrayQueue",
            &rt,
            ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO)
        );
        #[cfg(feature = "segmented-array")]
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue SegmentedArray 64",
            &rt,
            ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue VecDeque",
            &rt,
            ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO)
        );
        bench_zq_async_mpmc!(
            &mut group,
            "z-queue Swap<VecDeque, 2>",
            &rt,
            ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO)
        );
        group.finish();
    }
}

criterion_group!(
    benches,
    bench_unbounded_sync,
    bench_bounded_sync,
    bench_unbounded_async,
    bench_bounded_async,
);
criterion_main!(benches);
