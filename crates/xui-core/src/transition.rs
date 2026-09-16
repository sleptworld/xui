use xui_interface::ComputedStyle;

pub struct Transition {
    base_computed_style: ComputedStyle,
    to_style: ComputedStyle,
}

impl Transition {
    pub fn new(base_computed_style: ComputedStyle, to_style: ComputedStyle) -> Self {
        Self {
            base_computed_style,
            to_style,
        }
    }

    pub fn update(&mut self, base_computed_style: ComputedStyle, to_style: ComputedStyle) {
        self.base_computed_style = base_computed_style;
        self.to_style = to_style;
    }
}
