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
    }
}

pub type WidgetNodeState = WidgetState;
