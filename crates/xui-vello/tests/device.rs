//! The device an app's own GPU painters draw on.
//!
//! These need a GPU adapter; they return early without one, and where the
//! adapter lacks the features asked for there is nothing to compare.

use xui_cosmic::CosmicEngine;
use xui_vello::{DeviceNeeds, VelloBackend, VelloBackendOptions, wgpu};

fn backend(needs: &DeviceNeeds) -> Option<VelloBackend<CosmicEngine>> {
    VelloBackend::headless_with_device(1.0, VelloBackendOptions::default(), needs).ok()
}

#[test]
fn a_device_gives_painters_what_they_ask_where_the_adapter_can() {
    let Some(plain) = backend(&DeviceNeeds::default()) else {
        return;
    };
    let wanted = wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::FLOAT32_FILTERABLE;
    assert!(
        !plain.wgpu_context().device().features().intersects(wanted),
        "nothing is asked for unless the app does"
    );
    let limits = wgpu::Limits {
        max_binding_array_elements_per_shader_stage: 16,
        ..Default::default()
    };
    let needs = DeviceNeeds {
        features: wanted,
        limits: limits.clone(),
    };
    let asked = backend(&needs).expect("a device opens either way");
    let context = asked.wgpu_context();
    let device = context.device();
    if device.features().contains(wanted) {
        assert_eq!(
            device.limits().max_binding_array_elements_per_shader_stage,
            16
        );
    } else {
        // The adapter cannot give them: the device is the plain one.
        assert!(!device.features().intersects(wanted));
    }
}

#[test]
fn what_no_adapter_has_leaves_the_plain_device() {
    let needs = DeviceNeeds {
        features: wgpu::Features::all(),
        limits: wgpu::Limits::default(),
    };
    let Some(opened) = backend(&needs) else {
        return;
    };
    assert!(
        !opened
            .wgpu_context()
            .device()
            .features()
            .contains(wgpu::Features::all())
    );
}
