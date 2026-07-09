use num_traits::Zero;
use std::ops::{Add, AddAssign, Sub};

pub(super) struct Tree<I>
where
    I: Copy + Default + Zero + Add,
{
    inner: Vec<I>,
}

impl<I> Tree<I>
where
    I: Copy + Default + Zero + Add + AddAssign + Sub<Output = I>,
{
    fn new(n: usize) -> Self {
        Self {
            inner: vec![I::zero(); n + 1],
        }
    }

    fn lowbit(i: usize) -> usize {
        i & i.wrapping_neg()
    }

    fn add(&mut self, mut i: usize, delta: I) {
        while i < self.inner.len() {
            self.inner[i] += delta;
            i += Self::lowbit(i);
        }
    }

    fn prefix_sum(&self, mut i: usize) -> I {
        let mut sum = I::zero();

        while i > 0 {
            sum += self.inner[i];
            i -= Self::lowbit(i);
        }

        sum
    }

    fn range_sum(&self, l: usize, r: usize) -> I {
        self.prefix_sum(r) - self.prefix_sum(l - 1)
    }
}
