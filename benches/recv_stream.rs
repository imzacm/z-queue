#![cfg(feature = "stream")]

use std::num::NonZeroUsize;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use futures_util::StreamExt;
use tokio::runtime::Runtime;
#[cfg(feature = "segmented-array")]
use z_queue::container::SegmentedArray;
use z_queue::container::VecDeque;

// Note: Ensure these imports are also gated in your actual code if they
// depend on the features being enabled.
#[cfg(feature = "crossbeam-queue")]
use z_queue::container::{CrossbeamArrayQueue, CrossbeamSegQueue};

const ITEMS: usize = 10_000;
const PRODUCERS: usize = 4;
const CONSUMERS: usize = 4;
const BOUND: usize = 1000;

// --- Macros to reduce boilerplate across generic instantiations ---

macro_rules! spsc_bench {
    ($group:expr, $rt:expr, $name:expr, $init:expr) => {
        $group.bench_function($name, |b| {
            b.to_async(&$rt).iter(|| async {
                let (tx, rx) = $init;
                let mut stream = rx.into_stream();

                let producer = tokio::spawn(async move {
                    for i in 0..ITEMS {
                        let _ = tx.send_async(i).await;
                    }
                });

                while let Some(msg) = stream.next().await {
                    std::hint::black_box(msg);
                }
                producer.await.unwrap();
            });
        });
    };
}

macro_rules! mpsc_bench {
    ($group:expr, $rt:expr, $name:expr, $init:expr) => {
        $group.bench_function($name, |b| {
            b.to_async(&$rt).iter(|| async {
                let (tx, rx) = $init;
                let mut stream = rx.into_stream();
                let items_per_producer = ITEMS / PRODUCERS;

                for _ in 0..PRODUCERS {
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        for i in 0..items_per_producer {
                            let _ = tx_clone.send_async(i).await;
                        }
                    });
                }
                drop(tx);

                while let Some(msg) = stream.next().await {
                    std::hint::black_box(msg);
                }
            });
        });
    };
}

macro_rules! mpmc_bench {
    ($group:expr, $rt:expr, $name:expr, $init:expr) => {
        $group.bench_function($name, |b| {
            b.to_async(&$rt).iter(|| async {
                let (tx, rx) = $init;
                let items_per_producer = ITEMS / PRODUCERS;

                for _ in 0..PRODUCERS {
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        for i in 0..items_per_producer {
                            let _ = tx_clone.send_async(i).await;
                        }
                    });
                }
                drop(tx);

                let mut consumer_handles = Vec::with_capacity(CONSUMERS);
                for _ in 0..CONSUMERS {
                    let mut stream = rx.to_stream();
                    consumer_handles.push(tokio::spawn(async move {
                        while let Some(msg) = stream.next().await {
                            std::hint::black_box(msg);
                        }
                    }));
                }
                drop(rx);

                for handle in consumer_handles {
                    handle.await.unwrap();
                }
            });
        });
    };
}

// --- Benchmark Groups ---

fn bench_spsc(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("SPSC");
    group.throughput(Throughput::Elements(ITEMS as u64));
    let cap = NonZeroUsize::new(BOUND).unwrap();

    // --- Bounded ---
    #[cfg(feature = "crossbeam-queue")]
    spsc_bench!(
        group,
        rt,
        "z_queue/bounded/CrossbeamArrayQueue",
        z_queue::bounded::<CrossbeamArrayQueue<usize>>(cap)
    );

    #[cfg(feature = "segmented-array")]
    spsc_bench!(
        group,
        rt,
        "z_queue/bounded/SegmentedArray",
        z_queue::bounded::<SegmentedArray<usize, 64>>(cap)
    );

    spsc_bench!(group, rt, "z_queue/bounded/VecDeque", z_queue::bounded::<VecDeque<usize>>(cap));

    // --- Unbounded ---
    #[cfg(feature = "crossbeam-queue")]
    spsc_bench!(
        group,
        rt,
        "z_queue/unbounded/CrossbeamSegQueue",
        z_queue::unbounded::<CrossbeamSegQueue<usize>>()
    );

    #[cfg(feature = "segmented-array")]
    spsc_bench!(
        group,
        rt,
        "z_queue/unbounded/SegmentedArray",
        z_queue::unbounded::<SegmentedArray<usize, 64>>()
    );

    spsc_bench!(group, rt, "z_queue/unbounded/VecDeque", z_queue::unbounded::<VecDeque<usize>>());

    // --- Flume Baselines ---
    group.bench_function("flume/bounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::bounded(BOUND);
            let mut stream = rx.into_stream();
            let producer = tokio::spawn(async move {
                for i in 0..ITEMS {
                    let _ = tx.send_async(i).await;
                }
            });
            while let Some(msg) = stream.next().await {
                std::hint::black_box(msg);
            }
            producer.await.unwrap();
        });
    });

    group.bench_function("flume/unbounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::unbounded();
            let mut stream = rx.into_stream();
            let producer = tokio::spawn(async move {
                for i in 0..ITEMS {
                    let _ = tx.send_async(i).await;
                }
            });
            while let Some(msg) = stream.next().await {
                std::hint::black_box(msg);
            }
            producer.await.unwrap();
        });
    });

    group.finish();
}

fn bench_mpsc(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("MPSC");
    group.throughput(Throughput::Elements(ITEMS as u64));
    let cap = NonZeroUsize::new(BOUND).unwrap();

    // --- Bounded ---
    #[cfg(feature = "crossbeam-queue")]
    mpsc_bench!(
        group,
        rt,
        "z_queue/bounded/CrossbeamArrayQueue",
        z_queue::bounded::<CrossbeamArrayQueue<usize>>(cap)
    );

    #[cfg(feature = "segmented-array")]
    mpsc_bench!(
        group,
        rt,
        "z_queue/bounded/SegmentedArray",
        z_queue::bounded::<SegmentedArray<usize, 64>>(cap)
    );

    mpsc_bench!(group, rt, "z_queue/bounded/VecDeque", z_queue::bounded::<VecDeque<usize>>(cap));

    // --- Unbounded ---
    #[cfg(feature = "crossbeam-queue")]
    mpsc_bench!(
        group,
        rt,
        "z_queue/unbounded/CrossbeamSegQueue",
        z_queue::unbounded::<CrossbeamSegQueue<usize>>()
    );

    #[cfg(feature = "segmented-array")]
    mpsc_bench!(
        group,
        rt,
        "z_queue/unbounded/SegmentedArray",
        z_queue::unbounded::<SegmentedArray<usize, 64>>()
    );

    mpsc_bench!(group, rt, "z_queue/unbounded/VecDeque", z_queue::unbounded::<VecDeque<usize>>());

    // --- Flume Baselines ---
    let items_per_producer = ITEMS / PRODUCERS;
    group.bench_function("flume/bounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::bounded(BOUND);
            let mut stream = rx.into_stream();
            for _ in 0..PRODUCERS {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    for i in 0..items_per_producer {
                        let _ = tx_clone.send_async(i).await;
                    }
                });
            }
            drop(tx);
            while let Some(msg) = stream.next().await {
                std::hint::black_box(msg);
            }
        });
    });

    group.bench_function("flume/unbounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::unbounded();
            let mut stream = rx.into_stream();
            for _ in 0..PRODUCERS {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    for i in 0..items_per_producer {
                        let _ = tx_clone.send_async(i).await;
                    }
                });
            }
            drop(tx);
            while let Some(msg) = stream.next().await {
                std::hint::black_box(msg);
            }
        });
    });

    group.finish();
}

fn bench_mpmc(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("MPMC");
    group.throughput(Throughput::Elements(ITEMS as u64));
    let cap = NonZeroUsize::new(BOUND).unwrap();

    // --- Bounded ---
    #[cfg(feature = "crossbeam-queue")]
    mpmc_bench!(
        group,
        rt,
        "z_queue/bounded/CrossbeamArrayQueue",
        z_queue::bounded::<CrossbeamArrayQueue<usize>>(cap)
    );

    #[cfg(feature = "segmented-array")]
    mpmc_bench!(
        group,
        rt,
        "z_queue/bounded/SegmentedArray",
        z_queue::bounded::<SegmentedArray<usize, 64>>(cap)
    );

    mpmc_bench!(group, rt, "z_queue/bounded/VecDeque", z_queue::bounded::<VecDeque<usize>>(cap));

    // --- Unbounded ---
    #[cfg(feature = "crossbeam-queue")]
    mpmc_bench!(
        group,
        rt,
        "z_queue/unbounded/CrossbeamSegQueue",
        z_queue::unbounded::<CrossbeamSegQueue<usize>>()
    );

    #[cfg(feature = "segmented-array")]
    mpmc_bench!(
        group,
        rt,
        "z_queue/unbounded/SegmentedArray",
        z_queue::unbounded::<SegmentedArray<usize, 64>>()
    );

    mpmc_bench!(group, rt, "z_queue/unbounded/VecDeque", z_queue::unbounded::<VecDeque<usize>>());

    // --- Flume Baselines ---
    let items_per_producer = ITEMS / PRODUCERS;
    group.bench_function("flume/bounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::bounded(BOUND);
            for _ in 0..PRODUCERS {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    for i in 0..items_per_producer {
                        let _ = tx_clone.send_async(i).await;
                    }
                });
            }
            drop(tx);

            let mut consumer_handles = Vec::with_capacity(CONSUMERS);
            for _ in 0..CONSUMERS {
                let rx_clone = rx.clone();
                consumer_handles.push(tokio::spawn(async move {
                    let mut stream = rx_clone.into_stream();
                    while let Some(msg) = stream.next().await {
                        std::hint::black_box(msg);
                    }
                }));
            }
            drop(rx);
            for handle in consumer_handles {
                handle.await.unwrap();
            }
        });
    });

    group.bench_function("flume/unbounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::unbounded();
            for _ in 0..PRODUCERS {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    for i in 0..items_per_producer {
                        let _ = tx_clone.send_async(i).await;
                    }
                });
            }
            drop(tx);

            let mut consumer_handles = Vec::with_capacity(CONSUMERS);
            for _ in 0..CONSUMERS {
                let rx_clone = rx.clone();
                consumer_handles.push(tokio::spawn(async move {
                    let mut stream = rx_clone.into_stream();
                    while let Some(msg) = stream.next().await {
                        std::hint::black_box(msg);
                    }
                }));
            }
            drop(rx);
            for handle in consumer_handles {
                handle.await.unwrap();
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_spsc, bench_mpsc, bench_mpmc);
criterion_main!(benches);
