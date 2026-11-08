//! 同进程原生DLSS与外部NGX管理器共享生命周期时，显式转交最终Shutdown所有权。
use std::sync::atomic::{AtomicBool, Ordering}; // 退出时跨线程读取所有权，不增加渲染帧锁。
use std::sync::Mutex; // 只在SDK销毁和设备最终销毁边界登记，不进入帧关键路径。
use ash::vk::Handle as _; // 保存VkDevice位模式，不延长wgpu设备生命周期。

static EXTERNAL_OWNER: ShutdownOwnership = ShutdownOwnership::new(); // 默认维持上游独立NGX生命周期。
static DEFERRED_DEVICES: Mutex<Vec<u64>> = Mutex::new(Vec::new()); // 每个宿主NGX设备只登记一次，关闭后移出仓库。

struct ShutdownOwnership(AtomicBool); // 单向所有权登记，与进程期唯一Vulkan后端一致。
impl ShutdownOwnership { // 独立状态便于无SDK回归测试。
    const fn new() -> Self { Self(AtomicBool::new(false)) } // 未登记时必须自行关闭NGX。
    fn transfer(&self) { self.0.store(true, Ordering::Release); } // 外部生命周期完全建立后才发布。
    fn owns_shutdown(&self) -> bool { !self.0.load(Ordering::Acquire) } // 外部管理器存在时禁止提前关闭共享NGX。
} // 所有权记录结束。

/// 把最终NGX关闭交给已经成功初始化的进程期外部管理器。
///
/// # Safety
/// 调用者必须证明外部管理器已经成功初始化同一NGX Vulkan实例，并保证它在全部
/// SR/RR/FG功能释放后、VkDevice销毁前先关闭外部管理器全部插件，再调用
/// `shutdown_deferred_ngx_devices`关闭宿主静态链接的NGX loader；登记后不得在同进程卸载或
/// 替换该管理器。仅编译了FG、仅slInit成功或外部NGX初始化失败都不满足条件。
pub unsafe fn register_external_ngx_shutdown_owner() { // 显式能力交接，不能按Cargo特性猜测。
    EXTERNAL_OWNER.transfer(); // 单向登记，避免运行中反复转交导致双关闭。
} // 外部管理器登记结束。

pub(crate) fn owns_ngx_shutdown() -> bool { EXTERNAL_OWNER.owns_shutdown() } // DlssSdk仍必须释放其自身参数和功能。

pub(crate) fn defer_ngx_shutdown(device: ash::vk::Device) { // 调用方已释放全部宿主功能与参数，但外部FG仍可能使用NGX。
    let mut devices = DEFERRED_DEVICES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()); // 退出清理不因历史锁中毒直接放弃待释放句柄。
    register_device(&mut devices, device.as_raw()); // 不保留Device/Arc，保证真正的HAL最终销毁仍能到达。
} // SDK销毁阶段登记结束。

fn register_device(devices: &mut Vec<u64>, device: u64) { // 独立纯状态操作便于回归测试。
    if device != 0 && !devices.contains(&device) { devices.push(device); } // 同一设备多个SDK对象不得重复Shutdown1。
} // 登记函数结束。

/// 每个延迟关闭的真实NGX结果，交由拥有日志上下文的外部管理器记录。
pub struct DeferredNgxShutdownResult { // 不向调用者暴露静态链接绑定的内部枚举类型。
    pub device: u64, // 本次关闭的原始VkDevice身份。
    pub result: u32, // NVSDK_NGX_Result原始值，成功并不等于零。
    pub succeeded: bool, // 使用NGX自身成功判定，避免调用者误用Streamline返回码。
} // 可审计关闭结果结束。

impl DeferredNgxShutdownResult {
    /// Streamline 可以先关闭同设备的共享 NGX，宿主随后会收到未初始化状态。
    pub fn already_inactive(&self) -> bool {
        use crate::nvsdk_ngx::NVSDK_NGX_Result_NVSDK_NGX_Result_FAIL_NotInitialized;

        self.result == NVSDK_NGX_Result_NVSDK_NGX_Result_FAIL_NotInitialized as u32
    }
}

/// 在外部管理器及其全部NGX插件关闭后，释放宿主独立静态链接loader的设备状态。
///
/// # Safety
/// 调用者必须保证所有待登记SDK已Drop、没有并发NGX调用，且登记的VkDevice仍有效；
/// 必须在这些设备的vkDestroyDevice之前调用，并确保外部管理器已完成关闭。
pub unsafe fn shutdown_deferred_ngx_devices() -> Vec<DeferredNgxShutdownResult> { // 最终设备销毁回调使用，普通路径没有待处理项目。
    let devices = std::mem::take(&mut *DEFERRED_DEVICES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())); // 先取走所有权，多次回调只处理一次且不持锁调用SDK。
    devices.into_iter().map(|device| { // 每个独立设备只进行一次宿主loader关闭尝试。
        let result = unsafe { crate::nvsdk_ngx::NVSDK_NGX_VULKAN_Shutdown1(ash::vk::Device::from_raw(device)) }; // 必须在SL关闭后调用本库静态链接的入口。
        DeferredNgxShutdownResult { device, result: result as u32, succeeded: crate::nvsdk_ngx::check_ngx_result(result).is_ok() } // 保留原始结果与官方成功语义。
    }).collect() // 返回每项关闭事实；失败由管理器明确记录，不在无效设备销毁后重试。
} // 宿主延迟关闭结束。

#[cfg(test)] // 无GPU、无NVIDIA运行库即可检查默认及转交边界。
mod tests { // 使用局部状态，避免污染进程期生产标记。
    use super::{ShutdownOwnership, register_device}; // 测试与实际Drop共用的所有权和待关闭状态。
    #[test] // 同设备多SDK和重复最终回调都不能导致重复关闭。
    fn deferred_devices_are_deduplicated_and_drained() { // 不调用SDK、不污染生产全局状态。
        let mut devices = Vec::new(); // 局部待关闭设备列表。
        register_device(&mut devices, 7); // 首次登记有效设备。
        register_device(&mut devices, 7); // 重复SDK销毁登记同一设备。
        register_device(&mut devices, 0); // 空设备不可进入原生关闭。
        assert_eq!(std::mem::take(&mut devices), vec![7]); // 最终回调只取出一次。
        assert!(std::mem::take(&mut devices).is_empty()); // 重复回调幂等。
        register_device(&mut devices, 7); // 后续设备恢复可以重新使用相同原始句柄。
        assert_eq!(devices, vec![7]); // 不维护永久已关闭集合而误跳过新设备。
    } // 延迟关闭状态测试结束。
    #[test] // 普通非FG应用必须保持上游最终Shutdown。
    fn standalone_sdk_retains_shutdown() { // 不登记外部管理器。
        assert!(ShutdownOwnership::new().owns_shutdown()); // 不因编译链接了FrameGeneration而跳过释放。
    } // 独立路径测试结束。
    #[test] // 外部NGX完成初始化后禁止本库提前结束共享实例。
    fn transfer_is_one_way_and_idempotent() { // 多次确认不重新取得所有权。
        let ownership = ShutdownOwnership::new(); // 新的独立状态。
        ownership.transfer(); // 外部管理器接管最终关闭。
        assert!(!ownership.owns_shutdown()); // SR/RR释放不能销毁FG所需实例。
        ownership.transfer(); // 重复登记安全且不切回独立路径。
        assert!(!ownership.owns_shutdown()); // 最终关闭仍由外部管理器负责。
    } // 转交路径测试结束。
} // 所有权测试结束。
