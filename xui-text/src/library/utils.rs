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
        if name.len() <= self.buf.capacity() && name.is_ascii() {
            self.buf.clear();
            for c in name.bytes() {
                self.buf.push(c.to_ascii_lowercase() as char);
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

#[cfg(test)]
mod tests {
    use super::LowercaseString;

    #[test]
    fn lowercase_ascii_names_use_inline_buffer() {
        let mut lowercase = LowercaseString::new();

        assert_eq!(lowercase.get("System Font"), Some("system font"));
    }
}
