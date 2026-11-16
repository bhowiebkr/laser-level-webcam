//! The renderer that draws the desktop window. On Windows the app draws with Direct3D 12. Some
//! virtual machines and Remote Desktop sessions cannot open a window with it. Then the app tries
//! Direct3D 12 on the processor (WARP), then Vulkan or OpenGL. The `WGPU_BACKEND` variable
//! (`dx12`, `vulkan` or `gl`) sets one renderer and stops the other tries.

use std::fmt;
use std::sync::Arc;

use eframe::egui_wgpu::{WgpuSetup, wgpu};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Renderer {
    /// On Windows Direct3D 12, or the renderer that `WGPU_BACKEND` sets. On other systems the
    /// renderer that wgpu chooses.
    First,
    /// Direct3D 12 on the processor: the WARP adapter of Windows. It is slower, but it needs no
    /// graphics driver.
    Warp,
    /// Vulkan, or else OpenGL.
    VulkanGl,
}

impl fmt::Display for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::First if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() => "Direct3D 12",
            Self::First => "the first renderer",
            Self::Warp => "Direct3D 12 on the processor (WARP)",
            Self::VulkanGl => "Vulkan or OpenGL",
        })
    }
}

/// The renderers to try, in order.
pub fn order() -> &'static [Renderer] {
    if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
        &[Renderer::First, Renderer::Warp, Renderer::VulkanGl]
    } else {
        &[Renderer::First]
    }
}

/// Opens the window with each renderer in turn. A renderer that cannot open the window goes to
/// `failed`, and the next one tries. Other errors stop at once; the last renderer's error comes
/// back.
pub fn start(
    order: &[Renderer],
    mut open: impl FnMut(Renderer) -> eframe::Result,
    mut failed: impl FnMut(Renderer, &eframe::Error),
) -> eframe::Result {
    let (last, before) = order.split_last().expect("at least one renderer");
    for &r in before {
        match open(r) {
            Err(e @ eframe::Error::Wgpu(_)) => failed(r, &e),
            other => return other,
        }
    }
    open(*last)
}

/// Sets the window's wgpu options for this renderer.
pub fn apply(renderer: Renderer, options: &mut eframe::NativeOptions) {
    let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup else { return };
    let backends = &mut setup.instance_descriptor.backends;
    match renderer {
        Renderer::First => {
            if cfg!(windows) {
                *backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
            }
        }
        Renderer::Warp => {
            *backends = wgpu::Backends::DX12;
            setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
                adapters
                    .iter()
                    .find(|a| {
                        a.get_info().device_type == wgpu::DeviceType::Cpu
                            && surface.is_none_or(|s| a.is_surface_supported(s))
                    })
                    .cloned()
                    .ok_or_else(|| "Windows gave no WARP adapter".to_owned())
            }));
        }
        Renderer::VulkanGl => *backends = wgpu::Backends::VULKAN | wgpu::Backends::GL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_adapter() -> eframe::Error {
        eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::CustomNativeAdapterSelectionError("test".into()))
    }

    #[test]
    fn a_renderer_that_cannot_open_the_window_lets_the_next_one_try() {
        let all = [Renderer::First, Renderer::Warp, Renderer::VulkanGl];
        let (mut tried, mut failed) = (Vec::new(), Vec::new());
        let result = start(
            &all,
            |r| {
                tried.push(r);
                if r == Renderer::VulkanGl { Ok(()) } else { Err(no_adapter()) }
            },
            |r, _| failed.push(r),
        );
        assert!(result.is_ok());
        assert_eq!(tried, all);
        assert_eq!(failed, [Renderer::First, Renderer::Warp]);

        // The window opened: no more tries.
        let mut tried = Vec::new();
        assert!(
            start(
                &all,
                |r| {
                    tried.push(r);
                    Ok(())
                },
                |_, _| panic!("no renderer failed")
            )
            .is_ok()
        );
        assert_eq!(tried, [Renderer::First]);

        // All fail: the last error comes back, and only the ones before it go to `failed`.
        let mut failed = Vec::new();
        assert!(matches!(start(&all, |_| Err(no_adapter()), |r, _| failed.push(r)), Err(eframe::Error::Wgpu(_))));
        assert_eq!(failed, [Renderer::First, Renderer::Warp]);
    }

    #[test]
    fn other_errors_stop_the_tries() {
        let mut tried = 0;
        let result = start(
            &[Renderer::First, Renderer::Warp],
            |_| {
                tried += 1;
                Err(eframe::Error::AppCreation("the app".into()))
            },
            |_, _| panic!("not a renderer error"),
        );
        assert!(matches!(result, Err(eframe::Error::AppCreation(_))));
        assert_eq!(tried, 1);
    }

    #[test]
    fn each_renderer_sets_its_backends() {
        let backends = |r| {
            let mut options = eframe::NativeOptions::default();
            apply(r, &mut options);
            let WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup else { unreachable!() };
            (setup.instance_descriptor.backends, setup.native_adapter_selector.is_some())
        };
        assert_eq!(backends(Renderer::Warp), (wgpu::Backends::DX12, true));
        assert_eq!(backends(Renderer::VulkanGl), (wgpu::Backends::VULKAN | wgpu::Backends::GL, false));
        if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
            assert_eq!(backends(Renderer::First), (wgpu::Backends::DX12, false));
            assert_eq!(order().len(), 3);
        }
    }
}
