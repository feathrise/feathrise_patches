//! 仅为真实UNORM窗口补最终sRGB编码；离屏线性纹理、HDR和正常sRGB窗口保持原语义。
use bevy_camera::NormalizedRenderTarget; // 依据提取后的实际目标，避免把离屏UNORM图像误认为窗口。
use bevy_render::{camera::ExtractedCamera, render_resource::TextureFormat}; // 直接读取真实输出格式。
pub(super) fn requires_encoding(camera: Option<&ExtractedCamera>, format: TextureFormat) -> bool { // 与相机是否已经绘制无关，只看目标身份。
    select_encoding(camera.is_some_and(|camera| matches!(camera.target, Some(NormalizedRenderTarget::Window(_)))), format) // 不按全局开关猜测每个视图的输出。
} // 公开到upscaling父模块的决策入口结束。
fn select_encoding(window: bool, format: TextureFormat) -> bool { window && matches!(format, TextureFormat::Bgra8Unorm | TextureFormat::Rgba8Unorm) } // 只有这两种窗口编码与sRGB配对。
#[cfg(test)] // 纯决策测试无需GPU或窗口。
mod tests { // 保护纹理目标和HDR输出不被改变。
    use super::*; // 测试私有无副作用函数。
    #[test] // 覆盖目标与格式的组合。
    fn only_unorm_windows_need_encoding() { // 防止所有blit误加伽马编码。
        assert!(select_encoding(true, TextureFormat::Bgra8Unorm)); // 原生BGRA窗口需要编码。
        assert!(select_encoding(true, TextureFormat::Rgba8Unorm)); // 原生RGBA窗口需要编码。
        assert!(!select_encoding(false, TextureFormat::Bgra8Unorm)); // 离屏纹理必须保持线性。
        assert!(!select_encoding(true, TextureFormat::Bgra8UnormSrgb)); // 硬件sRGB视图不能二次编码。
        assert!(!select_encoding(true, TextureFormat::Rgba16Float)); // HDR输出不能使用SDR编码。
    } // 编码决策测试结束。
} // 测试模块结束。
