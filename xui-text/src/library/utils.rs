use std::sync::atomic::{AtomicU64, Ordering};

use crate::typ::SmallString;

pub struct LowercaseString {
    buf: SmallString,
    heap: String,
}

impl LowercaseString {
    pub fn new() -> Self {
        Self {
            buf: SmallString::new(),
            heap: Default::default(),
        }
    }

    pub fn get<'a>(&'a mut self, name: &str) -> Option<&'a str> {
        if name.len() <= self.buf.len() && name.is_ascii() {
            let mut end = 0;
            for c in name.as_bytes() {
                unsafe {
                    self.buf.as_bytes_mut()[end] = c.to_ascii_lowercase();
                }
                end += 1;
            }
            Some(self.buf.as_str())
        } else {
            self.heap = name.to_lowercase();
            Some(&self.heap)
        }
    }
}

pub struct AtomicCounter(AtomicU64);

impl AtomicCounter {
    pub const fn new() -> Self {
        Self(AtomicU64::new(1))
    }

    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}
