use core::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use event_listener::{Event, Listener};
use parking_lot::Mutex;

use crate::container::Container;

const MAX_SMALL_CAPACITY: usize = 1024;
const SEGMENT_SIZE: usize = 64;

#[derive(Debug)]
pub struct ZQueue<T> {
    container: CachePadded<Mutex<Container<T, SEGMENT_SIZE>>>,
    len: AtomicUsize,
    capacity: Option<usize>,
    // Notified on push.
    push_event: CachePadded<Event>,
    // Notified on pop.
    pop_event: CachePadded<Event>,
    // All waiters are notified on every push.
    find_waiter: CachePadded<Event>,
}

impl<T> ZQueue<T> {
    pub fn new<C>(capacity: C) -> Self
    where
        C: Into<Option<usize>>,
    {
        if let Some(capacity) = capacity.into() {
            Self::bounded(capacity)
        } else {
            Self::unbounded()
        }
    }

    pub fn bounded(capacity: usize) -> Self {
        let container = if capacity <= MAX_SMALL_CAPACITY {
            Container::new_vec_deque(capacity)
        } else {
            Container::new_segmented_array()
        };
        Self::new_inner(container, Some(capacity))
    }

    pub fn unbounded() -> Self {
        Self::new_inner(Container::new_segmented_array(), None)
    }

    fn new_inner(container: Container<T, SEGMENT_SIZE>, capacity: Option<usize>) -> Self {
        Self {
            container: CachePadded::new(Mutex::new(container)),
            len: AtomicUsize::new(0),
            capacity,
            push_event: CachePadded::new(Event::new()),
            pop_event: CachePadded::new(Event::new()),
            find_waiter: CachePadded::new(Event::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    pub fn is_empty(&self) -> bool {
        self.len.load(Ordering::Relaxed) == 0
    }

    pub fn is_full(&self) -> bool {
        self.capacity.is_some_and(|v| v <= self.len())
    }

    pub fn clear(&self) {
        self.container.lock().clear();
        self.len.store(0, Ordering::Relaxed);
        self.pop_event.notify(usize::MAX);
    }

    pub fn try_push(&self, item: T) -> bool {
        if self.is_full() {
            return false;
        }

        {
            let mut lock = self.container.lock();

            if self.is_full() {
                return false;
            }

            lock.push(item);
            self.len.fetch_add(1, Ordering::Relaxed);
        }

        self.push_event.notify(1);
        self.find_waiter.notify(usize::MAX);
        true
    }

    pub fn push(&self, item: T) {
        loop {
            let listener = self.pop_event.listen();

            {
                let mut lock = self.container.lock();

                if !self.is_full() {
                    lock.push(item);
                    self.len.fetch_add(1, Ordering::Relaxed);
                    drop(lock);
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
            }

            let backoff = crossbeam_utils::Backoff::new();
            while self.is_full() {
                if backoff.is_completed() {
                    listener.wait();
                    break;
                }
                backoff.snooze();
            }
        }
    }

    pub async fn push_async(&self, item: T) {
        loop {
            let listener = self.pop_event.listen();

            {
                let mut lock = self.container.lock();

                if !self.is_full() {
                    lock.push(item);
                    self.len.fetch_add(1, Ordering::Relaxed);
                    drop(lock);
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
            }

            listener.await;
        }
    }

    pub fn try_pop(&self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let item = self.container.lock().pop();
        if item.is_some() {
            self.len.fetch_sub(1, Ordering::Relaxed);
            self.pop_event.notify(1);
        }
        item
    }

    pub fn pop(&self) -> T {
        loop {
            let listener = self.push_event.listen();
            let item = self.container.lock().pop();
            if let Some(item) = item {
                self.len.fetch_sub(1, Ordering::Relaxed);
                self.pop_event.notify(1);
                return item;
            }

            let backoff = crossbeam_utils::Backoff::new();
            while self.is_empty() {
                if backoff.is_completed() {
                    listener.wait();
                    break;
                }
                backoff.snooze();
            }
        }
    }

    pub async fn pop_async(&self) -> T {
        loop {
            let listener = self.push_event.listen();
            let item = self.container.lock().pop();
            if let Some(item) = item {
                self.len.fetch_sub(1, Ordering::Relaxed);
                self.pop_event.notify(1);
                return item;
            }

            listener.await;
        }
    }

    pub fn try_find<F>(&self, find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        let item = self.container.lock().find_pop(find_fn);
        if item.is_some() {
            self.len.fetch_sub(1, Ordering::Relaxed);
            self.pop_event.notify(1);
        }
        item
    }

    pub fn find<F>(&self, mut find_fn: F) -> T
    where
        F: FnMut(&T) -> bool,
    {
        loop {
            let listener = self.find_waiter.listen();
            let item = self.container.lock().find_pop(&mut find_fn);
            if let Some(item) = item {
                self.len.fetch_sub(1, Ordering::Relaxed);
                self.pop_event.notify(1);
                return item;
            }

            listener.wait();
        }
    }

    pub async fn find_async<F>(&self, mut find_fn: F) -> T
    where
        F: FnMut(&T) -> bool,
    {
        loop {
            let listener = self.find_waiter.listen();
            let item = self.container.lock().find_pop(&mut find_fn);
            if let Some(item) = item {
                self.len.fetch_sub(1, Ordering::Relaxed);
                self.pop_event.notify(1);
                return item;
            }

            listener.await;
        }
    }

    pub fn retain<F>(&self, retain_fn: F)
    where
        F: FnMut(&T) -> bool,
    {
        let removed = self.container.lock().retain(retain_fn);
        self.len.fetch_sub(removed, Ordering::Relaxed);
        self.pop_event.notify(removed);
    }
}
