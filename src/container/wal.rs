use alloc::collections::VecDeque;
use alloc::vec::Vec;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use parking_lot::Mutex;

use crate::container::ContainerTrait;

#[derive(Debug)]
pub struct WalContainer<T, E, C> {
    wal: Mutex<Wal<T, E>>,
    container: C,
}

impl<T, E, C> WalContainer<T, E, C>
where
    T: PartialEq,
    E: Encoding<T>,
    C: ContainerTrait<T>,
{
    pub fn open<P>(path: P, encoding: E, container: C) -> Result<Self, std::io::Error>
    where
        P: AsRef<Path>,
    {
        let wal = Wal::open(path, encoding, &container)?;
        Ok(Self { wal: Mutex::new(wal), container })
    }
}

impl<T, E, C: ContainerTrait<T>> ContainerTrait<T> for WalContainer<T, E, C>
where
    T: PartialEq,
    E: Encoding<T>,
    C: ContainerTrait<T>,
{
    fn len(&self) -> usize {
        let lock = self.wal.lock();
        self.container.len() + lock.overflow.len()
    }

    fn capacity(&self) -> Option<usize> {
        self.container.capacity()
    }

    fn clear(&self) -> usize {
        self.container.clear()
    }

    fn push(&self, item: T) -> Result<(), T> {
        self.wal.lock().push(&item).expect("Failed to write push to WAL");
        let result = self.container.push(item);
        if let Err(item) = &result {
            self.wal.lock().pop(item).expect("Failed to write pop to WAL");
        }
        result
    }

    fn pop(&self) -> Option<T> {
        if let Some(item) = self.wal.lock().pop_overflow() {
            return Some(item);
        }

        let item = self.container.pop()?;
        self.wal.lock().pop(&item).expect("Failed to write pop to WAL");
        Some(item)
    }

    fn find_pop<F>(&self, mut find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        {
            let mut lock = self.wal.lock();
            let index = lock.overflow.iter().position(&mut find_fn);
            if let Some(index) = index {
                let item = lock.overflow.remove(index).unwrap();
                return Some(item);
            }
        }

        let item = self.container.find_pop(find_fn)?;
        self.wal.lock().pop(&item).expect("Failed to write pop to WAL");
        Some(item)
    }

    fn retain<F>(&self, retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let mut removed = Vec::new();
        self.retain_into(retain_fn, &mut removed);
        removed.len()
    }

    fn retain_into<F>(&self, retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
    {
        let old_len = removed.len();
        self.container.retain_into(retain_fn, removed);
        let new_len = removed.len();
        let removed_count = new_len - old_len;
        if removed_count == 0 {
            return;
        }

        let removed_slice = &removed[old_len..];
        for item in removed_slice {
            self.wal.lock().pop(item).expect("Failed to write pop to WAL");
        }
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        self.container.rand_shuffle(rng);
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        self.container.fastrand_shuffle();
    }
}

pub trait Encoding<T> {
    fn to_bytes(&self, item: &T, out: &mut Vec<u8>);

    #[allow(clippy::wrong_self_convention)]
    fn from_bytes(&self, bytes: &[u8]) -> Result<T, std::io::Error>;
}

#[derive(Debug)]
struct Wal<T, E> {
    file: File,
    // Items from WAL that didn't fit in queue.
    overflow: VecDeque<T>,
    encoding: E,
    buffer: Vec<u8>,
}

impl<T, E> Wal<T, E>
where
    T: PartialEq,
    E: Encoding<T>,
{
    fn open<P, C>(path: P, encoding: E, container: &C) -> Result<Self, std::io::Error>
    where
        P: AsRef<Path>,
        C: ContainerTrait<T>,
    {
        let (file, overflow) = open_wal(path.as_ref(), container, &encoding)?;
        Ok(Self { file, overflow, encoding, buffer: Vec::new() })
    }

    fn pop_overflow(&mut self) -> Option<T> {
        let item = self.overflow.pop_front()?;
        self.pop(&item).expect("Failed to write pop to WAL");
        if self.overflow.is_empty() {
            self.overflow = VecDeque::new();
        }
        Some(item)
    }

    fn write_operation(&mut self, op: u8, item: &T) -> Result<(), std::io::Error> {
        let mut bytes = [0u8; 4];
        bytes[0] = op;

        self.buffer.push(1);
        self.encoding.to_bytes(item, &mut self.buffer);

        let len = self.buffer.len() as u32;
        let len_bytes = len.to_le_bytes();
        bytes[1..].copy_from_slice(&len_bytes);

        self.file.write_all(&bytes)?;
        self.file.write_all(&self.buffer)?;
        self.file.flush()?;
        self.buffer.clear();
        Ok(())
    }

    fn push(&mut self, item: &T) -> Result<(), std::io::Error> {
        self.write_operation(1, item)
    }

    fn pop(&mut self, item: &T) -> Result<(), std::io::Error> {
        self.write_operation(2, item)
    }
}

fn open_wal<T, E, C>(
    path: &Path,
    container: &C,
    encoding: &E,
) -> Result<(File, VecDeque<T>), std::io::Error>
where
    T: PartialEq,
    E: Encoding<T>,
    C: ContainerTrait<T>,
{
    let mut items = VecDeque::new();
    let mut buffer = Vec::new();

    if let Ok(file) = File::open(path) {
        let mut reader = std::io::BufReader::new(file);

        loop {
            let mut op_buffer = [0u8; 1];
            match reader.read_exact(&mut op_buffer) {
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(error) => return Err(error),
            }

            let mut len_buffer = [0u8; 4];
            reader.read_exact(&mut len_buffer)?;
            let len = u32::from_le_bytes(len_buffer) as usize;
            buffer.reserve(len);

            reader.read_exact(&mut buffer)?;
            let item = encoding.from_bytes(&buffer)?;
            buffer.clear();

            match op_buffer[0] {
                1 => items.push_back(item),
                2 => {
                    let index = items.iter().position(|v| item.eq(v));
                    let Some(index) = index else { continue };
                    items.remove(index);
                }
                _ => return Err(std::io::Error::other("Unexpected operation in WAL")),
            }
        }
    }

    let temp_path = path.with_extension("temp-wal");
    let file = File::create(&temp_path)?;
    let mut writer = std::io::BufWriter::new(file);
    for item in &items {
        buffer.push(1);
        encoding.to_bytes(item, &mut buffer);
        writer.write_all(&buffer)?;
    }
    writer.flush()?;
    drop(writer);

    std::fs::rename(&temp_path, path)?;
    let mut file = File::options().write(true).create(false).truncate(false).open(path)?;
    file.seek(SeekFrom::End(0))?;

    while let Some(item) = items.pop_front() {
        if let Err(item) = container.push(item) {
            items.push_front(item);
        }
    }

    Ok((file, items))
}
