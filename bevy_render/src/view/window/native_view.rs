//! 显式窗口原生视图策略：代理交换链可能返回不支持跨格式视图的图像，末端编码交给upscaling。
use core::sync::atomic::{AtomicBool, Ordering}; // 进程期单向开关，读取无锁且不参与逐帧资源提取。
use wgpu::TextureFormat; // 与实际surface图像使用同一格式枚举。
static ENABLED: AtomicBool = AtomicBool::new(false); // 默认行为完全沿用上游sRGB窗口视图。
/// 必须在配置任何窗口surface前调用；启用后窗口视图保持图像原生格式。
pub fn use_native_window_view_formats() { ENABLED.store(true, Ordering::Release); } // 引导阶段幂等启用，不允许运行中切换已存在的surface。
pub(super) fn enabled() -> bool { ENABLED.load(Ordering::Acquire) } // 配置与取得图像遵循同一策略。
pub(super) fn view_format(format: TextureFormat) -> TextureFormat { select_format(format, enabled()) } // 对实际取得的图像格式作选择。
fn select_format(format: TextureFormat, native: bool) -> TextureFormat { if native { format } else { format.add_srgb_suffix() } } // 不制造代理图像并未声明的兼容视图。
#[cfg(test)] // 纯格式回归不触碰进程期开关。
mod tests { // 覆盖默认行为、原生UNORM以及HDR格式。
    use super::*; // 仅测试无副作用的决策函数。
    #[test] // 原生策略不得隐式添加sRGB后缀。
    fn native_views_preserve_image_format() { // 保留UNORM和HDR真实格式。
        assert_eq!(select_format(TextureFormat::Bgra8Unorm, true), TextureFormat::Bgra8Unorm); // DLSS-G常用输出。
        assert_eq!(select_format(TextureFormat::Rgba16Float, true), TextureFormat::Rgba16Float); // HDR不受影响。
        assert_eq!(select_format(TextureFormat::Bgra8Unorm, false), TextureFormat::Bgra8UnormSrgb); // 默认仍为原版行为。
    } // 格式测试结束。
} // 测试模块结束。
