use std::num::NonZeroUsize;
use std::sync::Arc;
use std::thread;

use criterion::{Criterion, criterion_group, criterion_main};
use tokio::runtime::Runtime;
use z_queue::ZQueue;
use z_queue::container::{CrossbeamArrayQueue, CrossbeamSegQueue, SegmentedArray, Swap, VecDeque};

const MESSAGES: usize = 1000;
const BOUND: usize = 100;
const BOUND_NON_ZERO: NonZeroUsize = NonZeroUsize::new(BOUND).unwrap();

// --- 1. UNBOUNDED SYNC BENCHMARKS ---
fn bench_unbounded_sync(c: &mut Criterion) {
    let mut group = c.benchmark_group("Unbounded Sync SPSC");

    group.bench_function("std::sync::mpsc", |b| {
        b.iter(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().unwrap());
            }
        });
    });

    group.bench_function("crossbeam_channel", |b| {
        b.iter(|| {
            let (tx, rx) = crossbeam_channel::unbounded();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().unwrap());
            }
        });
    });

    group.bench_function("flume", |b| {
        b.iter(|| {
            let (tx, rx) = flume::unbounded();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().unwrap());
            }
        });
    });

    group.bench_function("z-queue Crossbeam SegQueue", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<CrossbeamSegQueue<_>>::unbounded());
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.bench_function("z-queue SegmentedArray 64", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<SegmentedArray<_, 64>>::unbounded());
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.bench_function("z-queue VecDeque", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<VecDeque<_>>::unbounded());
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.bench_function("z-queue Swap<VecDeque, 2>", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<Swap<VecDeque<_>, 2>>::unbounded());
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.finish();
}

// --- 2. BOUNDED SYNC BENCHMARKS ---
fn bench_bounded_sync(c: &mut Criterion) {
    let mut group = c.benchmark_group("Bounded Sync SPSC");

    group.bench_function("std::sync::mpsc::sync_channel", |b| {
        b.iter(|| {
            let (tx, rx) = std::sync::mpsc::sync_channel(BOUND);
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().unwrap());
            }
        });
    });

    group.bench_function("crossbeam_channel", |b| {
        b.iter(|| {
            let (tx, rx) = crossbeam_channel::bounded(BOUND);
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().unwrap());
            }
        });
    });

    group.bench_function("flume", |b| {
        b.iter(|| {
            let (tx, rx) = flume::bounded(BOUND);
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().unwrap());
            }
        });
    });

    group.bench_function("z-queue Crossbeam ArrayQueue", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.bench_function("z-queue SegmentedArray 64", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.bench_function("z-queue VecDeque", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.bench_function("z-queue Swap<VecDeque, 2>", |b| {
        b.iter(|| {
            let queue = Arc::new(ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            thread::spawn(move || {
                for i in 0..MESSAGES {
                    queue_clone.push(i);
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop());
            }
        });
    });

    group.finish();
}

// --- 3. UNBOUNDED ASYNC BENCHMARKS ---
fn bench_unbounded_async(c: &mut Criterion) {
    let mut group = c.benchmark_group("Unbounded Async SPSC");
    let rt = Runtime::new().unwrap();

    group.bench_function("tokio::sync::mpsc", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    tx.send(i).unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().await.unwrap());
            }
        });
    });

    group.bench_function("flume", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::unbounded();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    tx.send_async(i).await.unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv_async().await.unwrap());
            }
        });
    });

    group.bench_function("z-queue Crossbeam SegQueue", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<CrossbeamSegQueue<_>>::unbounded());
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.bench_function("z-queue SegmentedArray 64", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<SegmentedArray<_, 64>>::unbounded());
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.bench_function("z-queue VecDeque", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<VecDeque<_>>::unbounded());
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.bench_function("z-queue Swap<VecDeque, 2>", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<Swap<VecDeque<_>, 2>>::unbounded());
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.finish();
}

// --- 4. BOUNDED ASYNC BENCHMARKS ---
fn bench_bounded_async(c: &mut Criterion) {
    let mut group = c.benchmark_group("Bounded Async SPSC");
    let rt = Runtime::new().unwrap();

    group.bench_function("tokio::sync::mpsc", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, mut rx) = tokio::sync::mpsc::channel(BOUND);
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    tx.send(i).await.unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv().await.unwrap());
            }
        });
    });

    group.bench_function("flume", |b| {
        b.to_async(&rt).iter(|| async {
            let (tx, rx) = flume::bounded(BOUND);
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    tx.send_async(i).await.unwrap();
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(rx.recv_async().await.unwrap());
            }
        });
    });

    group.bench_function("z-queue Crossbeam ArrayQueue", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<CrossbeamArrayQueue<_>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.bench_function("z-queue SegmentedArray 64", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<SegmentedArray<_, 64>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.bench_function("z-queue VecDeque", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<VecDeque<_>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.bench_function("z-queue Swap<VecDeque, 2>", |b| {
        b.to_async(&rt).iter(|| async {
            let queue = Arc::new(ZQueue::<Swap<VecDeque<_>, 2>>::bounded(BOUND_NON_ZERO));
            let queue_clone = queue.clone();
            tokio::spawn(async move {
                for i in 0..MESSAGES {
                    queue_clone.push_async(i).await;
                }
            });
            for _ in 0..MESSAGES {
                std::hint::black_box(queue.pop_async().await);
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_unbounded_sync,
    bench_bounded_sync,
    bench_unbounded_async,
    bench_bounded_async,
);
criterion_main!(benches);
