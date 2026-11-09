use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
    pub struct WidgetState: u8 {
        const HOVERED = 1 << 0;
        const PRESSED = 1 << 1;
        const FOCUSED = 1 << 2;
        const DRAGGING = 1 << 3;
        const DRAGING = Self::DRAGGING.bits();
        const DISABLED = 1 << 4;
        /// Focused in a way that should show a focus indicator, like CSS
        /// `:focus-visible`: by keyboard navigation, or programmatically right
        /// after keyboard input — not by a pointer press.
        const FOCUS_VISIBLE = 1 << 5;
        /// A drag from outside the window that this drop target accepts is
        /// over it.
        const DROP_OVER = 1 << 6;
    }
}

pub type WidgetNodeState = WidgetState;
