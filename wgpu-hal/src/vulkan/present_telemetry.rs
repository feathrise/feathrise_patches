//! 单设备、单交换链原型的只读 Present 事实；不占厂商回调，也不等待 GPU 或显示器。

use core::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Native Vulkan Present 被接受的进程累计值；不能解释为已扫描到屏幕的图像数量。
///
/// 本项目当前 guest 每进程只有一个设备和一个交换链。多个 Surface 并行使用时，本接口
/// 不提供逐 Surface 归属；调用者应使用对应呈现后端自己的遥测。
#[derive(Clone, Copy, Debug, Default)]
pub struct VulkanPresentTelemetry {
    pub successful_present_count: u64,
    /// 每次成功配置交换链递增；累计 Present 数不随重配归零。
    pub presentation_epoch: u64,
    pub width: u32,
    pub height: u32,
    /// 本次原子快照一致，且当前配置之后已有成功 Present；错误、释放和重配会撤销。
    pub ready: bool,
}

struct PresentCounters {
    sequence: AtomicU64,
    count: AtomicU64,
    epoch: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    ready: AtomicBool,
}

static COUNTERS: PresentCounters = PresentCounters::new();

impl PresentCounters {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            count: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            ready: AtomicBool::new(false),
        }
    }

    // 配置、Present 和释放原本就在同一 Surface 的写锁下串行；不新增互斥或等待。
    fn begin_write(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
    }

    fn end_write(&self) {
        self.sequence.fetch_add(1, Ordering::Release);
    }

    fn configured(&self, width: u32, height: u32) {
        self.begin_write();
        self.ready.store(false, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.width.store(width, Ordering::Relaxed);
        self.height.store(height, Ordering::Relaxed);
        self.end_write();
    }

    fn present_result(&self, success: bool) {
        self.begin_write();
        if success {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
        self.ready.store(success, Ordering::Relaxed);
        self.end_write();
    }

    fn released(&self) {
        self.begin_write();
        self.ready.store(false, Ordering::Relaxed);
        self.width.store(0, Ordering::Relaxed);
        self.height.store(0, Ordering::Relaxed);
        self.end_write();
    }

    fn snapshot(&self) -> VulkanPresentTelemetry {
        let before = self.sequence.load(Ordering::Acquire);
        let mut snapshot = VulkanPresentTelemetry {
            successful_present_count: self.count.load(Ordering::Relaxed),
            presentation_epoch: self.epoch.load(Ordering::Relaxed),
            width: self.width.load(Ordering::Relaxed),
            height: self.height.load(Ordering::Relaxed),
            ready: self.ready.load(Ordering::Relaxed),
        };
        // 不在渲染路径自旋重读；与写入冲突时只撤销这次样本的可用性。
        fence(Ordering::Acquire);
        let after = self.sequence.load(Ordering::Relaxed);
        snapshot.ready &= before == after && before & 1 == 0;
        snapshot
    }
}

/// 固定次数原子读取。`ready == false` 时，其他字段仅作诊断，不能证明当前配置已呈现。
pub fn present_telemetry() -> VulkanPresentTelemetry {
    COUNTERS.snapshot()
}

pub(super) fn configured(width: u32, height: u32) {
    COUNTERS.configured(width, height);
}

pub(super) fn present_result(success: bool) {
    COUNTERS.present_result(success);
}

pub(super) fn released() {
    COUNTERS.released();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_does_not_count_as_a_present() {
        let counters = PresentCounters::new();
        counters.configured(960, 540);
        let sample = counters.snapshot();
        assert_eq!(sample.presentation_epoch, 1);
        assert_eq!((sample.width, sample.height), (960, 540));
        assert_eq!(sample.successful_present_count, 0);
        assert!(!sample.ready);
        counters.present_result(true);
        assert!(counters.snapshot().ready);
        assert_eq!(counters.snapshot().successful_present_count, 1);
    }

    #[test]
    fn failure_and_release_preserve_monotonic_count_but_revoke_readiness() {
        let counters = PresentCounters::new();
        counters.configured(960, 540);
        counters.present_result(true);
        counters.present_result(false);
        let failed = counters.snapshot();
        assert_eq!(failed.successful_present_count, 1);
        assert!(!failed.ready);
        counters.released();
        let released = counters.snapshot();
        assert_eq!(released.successful_present_count, 1);
        assert_eq!((released.width, released.height), (0, 0));
        assert!(!released.ready);
        counters.configured(1280, 720);
        let resized = counters.snapshot();
        assert_eq!(resized.presentation_epoch, 2);
        assert_eq!(resized.successful_present_count, 1);
        assert!(!resized.ready);
        counters.present_result(true);
        assert_eq!(counters.snapshot().successful_present_count, 2);
        assert!(counters.snapshot().ready);
    }

    #[test]
    fn snapshot_does_not_wait_for_a_writer() {
        let counters = PresentCounters::new();
        counters.configured(960, 540);
        counters.present_result(true);
        counters.begin_write();
        assert!(!counters.snapshot().ready);
        counters.end_write();
        assert!(counters.snapshot().ready);
    }
}
