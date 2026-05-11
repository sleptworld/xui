bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DirtyFlags: u8 {
        const STATE = 1 << 0;
        const PROPS = 1 << 1;
        const STYLE = 1 << 2;
        const LAYOUT = 1 << 3;
        const PAINT = 1 << 4;
        const TREE = 1 << 5;
    }
}

impl Default for DirtyFlags {
    fn default() -> Self {
        Self::LAYOUT | Self::PAINT
    }
}
