use crate::{feature_info::with_feature_info, nvsdk_ngx::*};
use std::{
    ptr,
    sync::{Arc, Mutex},
    thread,
};
use uuid::Uuid;
use wgpu::{Device, hal::api::Vulkan};

/// Application-wide DLSS object.
pub struct DlssSdk {
    pub(crate) parameters: *mut NVSDK_NGX_Parameter,
    pub(crate) device: Device,
}

impl DlssSdk {
    /// Creates the DLSS SDK.
    ///
    /// This should be done once per application.
    pub fn new(project_id: Uuid, device: Device) -> Result<Arc<Mutex<Self>>, DlssError> {
        check_for_updates(project_id);

        let mut parameters = ptr::null_mut();
        unsafe {
            let hal_device = device.as_hal::<Vulkan>().unwrap();
            let shared_instance = hal_device.shared_instance();
            let raw_instance = shared_instance.raw_instance();

            let init_result = with_feature_info(project_id, Default::default(), |feature_info| {
                NVSDK_NGX_VULKAN_Init_with_ProjectID(
                    feature_info.Identifier.v.ProjectDesc.ProjectId,
                    NVSDK_NGX_EngineType_NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                    feature_info.Identifier.v.ProjectDesc.EngineVersion,
                    feature_info.ApplicationDataPath,
                    raw_instance.handle(),
                    hal_device.raw_physical_device(),
                    hal_device.raw_device().handle(),
                    shared_instance.entry().static_fn().get_instance_proc_addr,
                    raw_instance.fp_v1_0().get_device_proc_addr,
                    feature_info.FeatureInfo,
                    NVSDK_NGX_Version_NVSDK_NGX_Version_API,
                )
            });
            if let Err(error) = check_ngx_result(init_result) {
                report_initialization_failure(
                    "VULKAN_Init_with_ProjectID",
                    init_result,
                    [Some(init_result), None, None],
                    None,
                );
                return Err(error);
            }

            let capability_result = NVSDK_NGX_VULKAN_GetCapabilityParameters(&mut parameters);
            if let Err(error) = check_ngx_result(capability_result) {
                report_initialization_failure(
                    "VULKAN_GetCapabilityParameters",
                    capability_result,
                    [Some(init_result), Some(capability_result), None],
                    None,
                );
                return Err(error);
            }

            let mut dlss_supported = 0;
            let available_result = NVSDK_NGX_Parameter_GetI(
                parameters,
                NVSDK_NGX_Parameter_SuperSampling_Available.as_ptr().cast(),
                &mut dlss_supported,
            );
            let result = check_ngx_result(available_result);
            if result.is_err() {
                report_initialization_failure(
                    "Parameter_GetI(SuperSampling.Available)",
                    available_result,
                    [
                        Some(init_result),
                        Some(capability_result),
                        Some(available_result),
                    ],
                    None,
                );
                check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(parameters))?;
                result?;
            }
            if dlss_supported == 0 {
                // 设备扩展已就绪仍可能被NGX能力检查拒绝；只在此启动失败边界查询官方诊断键。
                let diagnostics = unavailable_diagnostics(parameters);
                report_initialization_failure(
                    "SuperSampling.Available=0",
                    available_result,
                    [
                        Some(init_result),
                        Some(capability_result),
                        Some(available_result),
                    ],
                    Some(&diagnostics),
                );
                check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(parameters))?;
                return Err(DlssError::FeatureNotSupported);
            }
        }

        Ok(Arc::new(Mutex::new(Self { parameters, device })))
    }

    /// Returns the number of bytes of VRAM allocated by DLSS.
    pub fn get_vram_allocated_bytes(&mut self) -> Result<u64, DlssError> {
        let mut vram_allocated_bytes = 0;
        check_ngx_result(unsafe {
            NGX_DLSS_GET_STATS(self.parameters, &mut vram_allocated_bytes)
        })?;
        Ok(vram_allocated_bytes)
    }
}

/// 仅失败初始化输出一次结构化边界，不改能力门、SDK身份、更新策略或帧内运行行为。
fn report_initialization_failure(
    stage: &str,
    raw_result: NVSDK_NGX_Result,
    stages: [Option<NVSDK_NGX_Result>; 3],
    detail: Option<&str>,
) {
    #[cfg(target_os = "windows")]
    let runtime_directory = crate::feature_info::approved_windows_runtime_directory();
    #[cfg(not(target_os = "windows"))]
    let runtime_directory: Option<std::path::PathBuf> = None;
    let executable = std::env::current_exe().ok();
    let [init_result, capability_result, available_result] =
        stages.map(|value| value.map(|value| format!("0x{value:08x}")));
    eprintln!(
        "DLSS initialization diagnostic stage={stage:?} raw_result=0x{raw_result:08x} init_result={init_result:?} capability_result={capability_result:?} available_result={available_result:?} approved_directory={runtime_directory:?} executable={executable:?} {}",
        detail.unwrap_or("")
    );
}

/// 查询失败同样保留原始结果码，不能把未返回的驱动版本或初始化结果当成有效零值。
unsafe fn unavailable_diagnostics(parameters: *mut NVSDK_NGX_Parameter) -> String {
    let mut needs_updated_driver = -1;
    let needs_result = unsafe {
        NVSDK_NGX_Parameter_GetI(
            parameters,
            NVSDK_NGX_Parameter_SuperSampling_NeedsUpdatedDriver
                .as_ptr()
                .cast(),
            &mut needs_updated_driver,
        )
    };
    let mut minimum_major = 0;
    let major_result = unsafe {
        NVSDK_NGX_Parameter_GetUI(
            parameters,
            NVSDK_NGX_Parameter_SuperSampling_MinDriverVersionMajor
                .as_ptr()
                .cast(),
            &mut minimum_major,
        )
    };
    let mut minimum_minor = 0;
    let minor_result = unsafe {
        NVSDK_NGX_Parameter_GetUI(
            parameters,
            NVSDK_NGX_Parameter_SuperSampling_MinDriverVersionMinor
                .as_ptr()
                .cast(),
            &mut minimum_minor,
        )
    };
    let mut feature_init_result = 0;
    let feature_result = unsafe {
        NVSDK_NGX_Parameter_GetI(
            parameters,
            NVSDK_NGX_Parameter_SuperSampling_FeatureInitResult
                .as_ptr()
                .cast(),
            &mut feature_init_result,
        )
    };
    format!(
        "needs_updated_driver={{result:0x{needs_result:08x},value:{needs_updated_driver}}} minimum_driver={{major_result:0x{major_result:08x},major:{minimum_major},minor_result:0x{minor_result:08x},minor:{minimum_minor}}} feature_init={{result:0x{feature_result:08x},value:0x{:08x}}}",
        feature_init_result as u32,
    )
}

fn check_for_updates(project_id: Uuid) {
    thread::spawn(move || {
        with_feature_info(
            project_id,
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_SuperSampling,
            |feature_info| unsafe {
                NVSDK_NGX_UpdateFeature(&feature_info.Identifier, feature_info.FeatureID);
            },
        );
        with_feature_info(
            project_id,
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_RayReconstruction,
            |feature_info| unsafe {
                NVSDK_NGX_UpdateFeature(&feature_info.Identifier, feature_info.FeatureID);
            },
        );
    });
}

impl Drop for DlssSdk {
    fn drop(&mut self) {
        unsafe {
            let hal_device = self.device.as_hal::<Vulkan>().unwrap();
            hal_device
                .raw_device()
                .device_wait_idle()
                .expect("Failed to wait for idle device when destroying DlssSdk");

            check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(self.parameters))
                .expect("Failed to destroy DlssSdk parameters");
            if crate::lifecycle::owns_ngx_shutdown() {
                // 仅独立路径关闭NGX；共享SL路径必须等FG功能全部释放。
                check_ngx_result(NVSDK_NGX_VULKAN_Shutdown1(hal_device.raw_device().handle()))
                    .expect("Failed to destroy DlssSdk"); // 普通未登记外部管理器时保留原释放语义。
            } else {
                // SL仍有FG对象时不能提前关闭宿主loader，但也不能永久跳过其独立静态状态释放。
                crate::lifecycle::defer_ngx_shutdown(hal_device.raw_device().handle()); // 仅登记裸设备，最终SL关闭后再执行本库Shutdown1。
            } // 普通路径维持原语义，共享路径转入可审计延迟关闭。
        }
    }
}

unsafe impl Send for DlssSdk {}
unsafe impl Sync for DlssSdk {}
