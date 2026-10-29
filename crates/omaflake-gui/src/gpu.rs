// SPDX-License-Identifier: AGPL-3.0-or-later
//! Which graphics API the window draws with. iced draws with wgpu, which on Linux prefers Vulkan; in a VM without a
//! Vulkan GPU (virgl – every omaflake NixOS guest – and many others) the only Vulkan device is Mesa's software
//! rasterizer (lavapipe), which keeps a CPU core busy while OpenGL (virgl) is accelerated. So before the window
//! opens, the app asks Vulkan which adapters there are; when every one is a CPU, it sets `WGPU_BACKEND=gl` for itself
//! (iced's wgpu compositor reads it). A `WGPU_BACKEND` the user set is left alone. Should OpenGL fail too, iced falls
//! back to its software renderer (tiny-skia).

use wgpu::DeviceType;

/// The device types of Vulkan's adapters call for OpenGL: there are some, and all of them are CPUs (lavapipe,
/// llvmpipe). No Vulkan at all is left to wgpu, which tries OpenGL by itself.
pub fn prefer_gl(vulkan: &[DeviceType]) -> bool {
    !vulkan.is_empty() && vulkan.iter().all(|t| *t == DeviceType::Cpu)
}

/// Sets `WGPU_BACKEND=gl` when Vulkan offers only a software adapter and the user chose nothing; Linux only. Returns
/// whether it did (for the log).
pub fn avoid_software_vulkan() -> bool {
    if !cfg!(target_os = "linux") || std::env::var_os("WGPU_BACKEND").is_some() {
        return false;
    }
    let types = vulkan_device_types();
    if !prefer_gl(&types) {
        return false;
    }
    // SAFETY: called at the start of main, before iced, tokio or any other thread runs, so nothing reads the
    // environment concurrently
    unsafe { std::env::set_var("WGPU_BACKEND", "gl") };
    true
}

/// The device types of the Vulkan adapters (none when there is no Vulkan loader or driver)
fn vulkan_device_types() -> Vec<DeviceType> {
    let instance =
        wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..Default::default() });
    instance.enumerate_adapters(wgpu::Backends::VULKAN).iter().map(|a| a.get_info().device_type).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_software_vulkan_means_gl() {
        assert!(prefer_gl(&[DeviceType::Cpu]));
        assert!(!prefer_gl(&[DeviceType::Cpu, DeviceType::IntegratedGpu]));
        assert!(!prefer_gl(&[DeviceType::DiscreteGpu]));
        assert!(!prefer_gl(&[DeviceType::VirtualGpu]));
        assert!(!prefer_gl(&[]));
    }
}
