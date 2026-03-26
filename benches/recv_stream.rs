#![cfg(feature = "stream")]

use std::num::NonZeroUsize;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use futures_util::StreamExt;
use tokio::runtime::Runtime;
use z_queue::defaults::{bounded as z_bounded, unbounded as z_unbounded};

const ITEMS: usize = 10_000;
const PRODUCERS: usize = 4;
const CONSUMERS: usize = 4;
const BOUND: usize = 1000;

fn bench_spsc(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("SPSC");
    group.throughput(Throughput::Elements(ITEMS as u64));

    // --- Bounded ---
    group.bench_function("z_queue/bounded", |b| {
        b.to_async(&rt).iter(|| async {
            let capacity = NonZeroUsize::new(BOUND).unwrap();
            let (tx, rx) = z_bounded(capacity);
            let mut stream = rx.into_recv_stream();

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

    // --- Unbounded ---
    group.bench_function("z_queue/unbounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = z_unbounded();
            let mut stream = rx.into_recv_stream();

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

    let items_per_producer = ITEMS / PRODUCERS;

    // --- Bounded ---
    group.bench_function("z_queue/bounded", |b| {
        b.to_async(&rt).iter(|| async {
            let capacity = NonZeroUsize::new(BOUND).unwrap();
            let (tx, rx) = z_bounded(capacity);
            let mut stream = rx.into_recv_stream();

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

    // --- Unbounded ---
    group.bench_function("z_queue/unbounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = z_unbounded();
            let mut stream = rx.into_recv_stream();

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

    let items_per_producer = ITEMS / PRODUCERS;

    // --- Bounded ---
    group.bench_function("z_queue/bounded", |b| {
        b.to_async(&rt).iter(|| async {
            let capacity = NonZeroUsize::new(BOUND).unwrap();
            let (tx, rx) = z_bounded(capacity);

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
                let mut stream = rx.recv_stream(); // Uses your internal Arc clone
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
                let rx_clone = rx.clone(); // Flume requires cloning the receiver
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

    // --- Unbounded ---
    group.bench_function("z_queue/unbounded", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = z_unbounded();

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
                let mut stream = rx.recv_stream();
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
