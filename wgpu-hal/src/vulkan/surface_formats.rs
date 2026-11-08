//! 显式启动的UNORM交换链偏好；只收窄真实Surface能力，最终窗口色彩编码由上层输出策略负责。
use alloc::vec::Vec; // 原始能力集合已由Vulkan查询构造，不增加帧内集合。
use core::sync::atomic::{AtomicBool, Ordering}; // 进程期只启用一次，Surface查询不等待锁。
use wgt::{SurfaceColorSpaces, SurfaceFormatCapabilities, TextureFormat};
static PREFER_UNORM: AtomicBool = AtomicBool::new(false); // 普通运行与FSR未调用入口时保持驱动原始能力及顺序。

/// 在创建任何Surface或查询其能力之前显式启用UNORM交换链偏好；重复调用幂等，进程内不撤销。
pub fn prefer_unorm_surface_formats() { PREFER_UNORM.store(true, Ordering::Release); } // 仅DLSS-G引导调用，不根据DLL存在或显卡型号猜策略。
pub(super) fn streamline_surface_requested() -> bool { PREFER_UNORM.load(Ordering::Acquire) } // 原生交换链仅在同一显式DLSS启动路径补充必需复制用途。

pub(super) fn apply(formats: &mut Vec<SurfaceFormatCapabilities>) {
    filter_paired_srgb(formats, streamline_surface_requested());
}

fn filter_paired_srgb(formats: &mut Vec<SurfaceFormatCapabilities>, enabled: bool) {
    if !enabled {
        return;
    }

    // WGPU 30 按格式列出可用色彩空间，替代格式必须覆盖被移除格式的全部能力。
    let color_spaces = |format| {
        formats
            .iter()
            .find(|caps| caps.format == format)
            .map_or(SurfaceColorSpaces::empty(), |caps| caps.color_spaces)
    };
    let rgba = color_spaces(TextureFormat::Rgba8Unorm);
    let bgra = color_spaces(TextureFormat::Bgra8Unorm);
    formats.retain(|caps| match caps.format {
        TextureFormat::Rgba8UnormSrgb => !rgba.contains(caps.color_spaces),
        TextureFormat::Bgra8UnormSrgb => !bgra.contains(caps.color_spaces),
        _ => true,
    });
}

#[cfg(test)]
mod tests {
    use super::filter_paired_srgb;
    use alloc::vec;
    use wgt::{SurfaceColorSpaces, SurfaceFormatCapabilities, TextureFormat, TextureFormat::*};

    fn srgb(format: TextureFormat) -> SurfaceFormatCapabilities {
        SurfaceFormatCapabilities {
            format,
            color_spaces: SurfaceColorSpaces::SRGB,
        }
    }

    #[test]
    fn default_does_not_change_surface_formats() {
        let original = vec![srgb(Bgra8UnormSrgb), srgb(Rgba8UnormSrgb), srgb(Bgra8Unorm), srgb(Rgba8Unorm)];
        let mut formats = original.clone();
        filter_paired_srgb(&mut formats, false);
        assert_eq!(formats, original);
    }

    #[test]
    fn enabled_removes_only_srgb_with_supported_unorm_pair() {
        let mut formats = vec![srgb(Bgra8UnormSrgb), srgb(Rgb10a2Unorm), srgb(Rgba8UnormSrgb), srgb(Bgra8Unorm), srgb(Rgba8Unorm)];
        filter_paired_srgb(&mut formats, true);
        assert_eq!(formats, vec![srgb(Rgb10a2Unorm), srgb(Bgra8Unorm), srgb(Rgba8Unorm)]);
    }

    #[test]
    fn unsupported_pairs_and_single_srgb_remain_available() {
        let mut formats = vec![
            srgb(Bgra8UnormSrgb),
            SurfaceFormatCapabilities {
                format: Bgra8Unorm,
                color_spaces: SurfaceColorSpaces::DISPLAY_P3,
            },
            srgb(Rgba8Unorm),
        ];
        let original = formats.clone();
        filter_paired_srgb(&mut formats, true);
        assert_eq!(formats, original);
        let mut only = vec![srgb(Rgba8UnormSrgb)];
        filter_paired_srgb(&mut only, true);
        assert_eq!(only, vec![srgb(Rgba8UnormSrgb)]);
    }
}
