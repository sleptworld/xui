bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct WidgetUpdateFlags: u8 {
        const STYLE_TARGET = 1 << 0;
        const LAYOUT_INPUT = 1 << 1;
        const PAINT_OUTPUT = 1 << 2;
        const STATE_CHANGE = 1 << 3;
        const TEXT_SHAPE = 1 << 4;
        const TREE = 1 << 5;
    }
}

impl Default for WidgetUpdateFlags {
    fn default() -> Self {
        Self::LAYOUT_INPUT | Self::PAINT_OUTPUT
    }
}

bitflags::bitflags! {
    /// Which parts of a [`ComputedStyle`](crate::ComputedStyle) differ.
    ///
    /// Every bit but `INHERITED` names a property group and says what the node
    /// itself has to redo. `INHERITED` cuts across the groups: it is set when
    /// any property descendants inherit changed, so it is the only bit that
    /// says anything about the subtree.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct StyleDiffFlags: u8 {
        const TEXT = 1 << 0;
        const LAYOUT = 1 << 1;
        const PAINT = 1 << 2;
        const SCROLL = 1 << 3;
        const EFFECT = 1 << 4;
        const TRANSFORM = 1 << 5;
        const INHERITED = 1 << 6;
    }
}
