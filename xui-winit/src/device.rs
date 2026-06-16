use xui_interface::events::XuiDeviceId;

#[derive(Default)]
pub struct WinitDeviceRegistry {
    next: u32,
    map: std::collections::HashMap<winit::event::DeviceId, XuiDeviceId>,
}

impl WinitDeviceRegistry {
    pub fn get_or_insert(&mut self, id: winit::event::DeviceId) -> XuiDeviceId {
        *self.map.entry(id).or_insert_with(|| {
            let xui_id = XuiDeviceId::new(self.next);
            self.next += 1;
            xui_id
        })
    }
}
