use xui::prelude::*;
use xui::components::*;

component_fn! {
    fn badge() {
        let state = cx.use_state(|| {3});
        <label color={Color::BLACK}>{format!("Badge: {}", state.get())}</label>
    }

    pub fn summary() {
        <column gap={4.0}>
            <badge />
            <label color={Color::BLUE_500}>{"defined with multiple component_fn functions"}</label>
        </column>
    }
}
