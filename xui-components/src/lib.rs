pub mod button;

pub fn register_components(registry: &mut xui::ComponentRegistry) {
    button::register_components(registry);
}
