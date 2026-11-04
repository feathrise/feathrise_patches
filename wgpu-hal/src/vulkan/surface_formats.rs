//! 显式启动的UNORM交换链偏好；只收窄真实Surface能力，最终窗口色彩编码由上层输出策略负责。
use alloc::vec::Vec; // 原始能力集合已由Vulkan查询构造，不增加帧内集合。
use core::sync::atomic::{AtomicBool, Ordering}; // 进程期只启用一次，Surface查询不等待锁。
static PREFER_UNORM: AtomicBool = AtomicBool::new(false); // 普通运行与FSR未调用入口时保持驱动原始能力及顺序。

/// 在创建任何Surface或查询其能力之前显式启用UNORM交换链偏好；重复调用幂等，进程内不撤销。
pub fn prefer_unorm_surface_formats() { PREFER_UNORM.store(true, Ordering::Release); } // 仅DLSS-G引导调用，不根据DLL存在或显卡型号猜策略。
pub(super) fn streamline_surface_requested() -> bool { PREFER_UNORM.load(Ordering::Acquire) } // 原生交换链仅在同一显式DLSS启动路径补充必需复制用途。

pub(super) fn apply(formats: &mut Vec<wgt::TextureFormat>) { // 仍由原生Surface能力查询拥有数据。
    filter_paired_srgb(formats, PREFER_UNORM.load(Ordering::Acquire)); // 只读取固定开关，未启用立即返回。
} // 能力策略应用结束。

fn filter_paired_srgb(formats: &mut Vec<wgt::TextureFormat>, enabled: bool) { // 纯函数保持没有UNORM替代项的有效sRGB能力。
    if !enabled { return; } // 默认不改任何格式或排序。
    let rgba = formats.contains(&wgt::TextureFormat::Rgba8Unorm); // 仅认可驱动实际报告的对应通道格式。
    let bgra = formats.contains(&wgt::TextureFormat::Bgra8Unorm); // BGRA不能用RGBA替代来假装支持。
    formats.retain(|format| match format { // 保持所有保留项的原始相对顺序，不引入新格式。
        wgt::TextureFormat::Rgba8UnormSrgb => !rgba, // 有RGBA UNORM才能移除配对sRGB能力。
        wgt::TextureFormat::Bgra8UnormSrgb => !bgra, // 有BGRA UNORM才能移除配对sRGB能力。
        _ => true, // HDR和其他真实支持格式保持不变。
    }); // 不伪造格式、不重写Vulkan创建参数，不会把唯一候选过滤为空。
} // 纯格式过滤结束。

#[cfg(test)] // 不启动Vulkan、不修改进程全局开关的纯能力回归。
mod tests { // 精确覆盖默认路径与稀疏驱动能力。
    use super::filter_paired_srgb; // 生产使用的唯一筛选逻辑。
    use alloc::vec; // 构造固定小型测试能力集合。
    use wgt::TextureFormat::*; // 用格式名称表达配对关系。
    #[test] // 未请求DLSS-G时必须逐项保持原始能力及顺序。
    fn default_does_not_change_surface_formats() { // FSR和普通路径不受影响。
        let original = vec![Bgra8UnormSrgb, Rgba8UnormSrgb, Bgra8Unorm, Rgba8Unorm]; // 常见sRGB优先驱动枚举。
        let mut formats = original.clone(); filter_paired_srgb(&mut formats, false); // 显式关闭策略。
        assert_eq!(formats, original); // 内容及顺序完全一致。
    } // 默认行为回归结束。
    #[test] // 两种配对sRGB均移除，UNORM与HDR保留真实顺序。
    fn enabled_removes_only_srgb_with_supported_unorm_pair() { // 不把HDR交换链能力一并删除。
        let mut formats = vec![Bgra8UnormSrgb, Rgb10a2Unorm, Rgba8UnormSrgb, Bgra8Unorm, Rgba8Unorm]; // 驱动提供两种完整配对。
        filter_paired_srgb(&mut formats, true); // DLSS-G显式策略。
        assert_eq!(formats, vec![Rgb10a2Unorm, Bgra8Unorm, Rgba8Unorm]); // 不添加、不重排，仅过滤配对项。
    } // 配对格式回归结束。
    #[test] // 缺少相同通道UNORM时不能捏造驱动能力或清空唯一格式。
    fn unsupported_pairs_and_single_srgb_remain_available() { // 有RGBA不意味着支持BGRA UNORM。
        let mut formats = vec![Bgra8UnormSrgb, Rgba8Unorm]; filter_paired_srgb(&mut formats, true); // 只有另一通道UNORM。
        assert_eq!(formats, vec![Bgra8UnormSrgb, Rgba8Unorm]); // 不跨通道误过滤。
        let mut only = vec![Rgba8UnormSrgb]; filter_paired_srgb(&mut only, true); // 唯一候选必须保留。
        assert_eq!(only, vec![Rgba8UnormSrgb]); // 能力集合仍有效，后端自行报告不支持。
    } // 不伪造能力回归结束。
} // 纯函数测试结束。
