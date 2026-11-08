//! NVIDIA Deep Learning Super Sampling (DLSS).
//!
//! DLSS uses machine learning models to upscale and anti-alias images.
//!
//! Requires a NVIDIA RTX GPU, and the Windows/Linux Vulkan rendering backend. Does not work on other platforms.
//!
//! See https://github.com/bevyengine/dlss_wgpu for licensing requirements and setup instructions.
//!
//! # Usage
//! 1. Enable Bevy's `dlss` feature
//! 2. During app setup, insert the `DlssProjectId` resource before `DefaultPlugins`
//! 3. Check for the presence of `Option<Res<DlssSuperResolutionSupported>>` at runtime to see if DLSS is supported on the current machine
//! 4. Add the `Dlss` component to your camera entity, optionally setting a specific `DlssPerfQualityMode` (defaults to `Auto`)
//! 5. Optionally add sharpening via `ContrastAdaptiveSharpening`
//! 6. Custom rendering code, including third party crates, should account for the optional `MainPassResolutionOverride` to work with DLSS (see the `custom_render_phase` example)

mod extract;
mod input_color_debug;
mod node;
mod prepare;

pub use dlss_wgpu::DlssPerfQualityMode;
pub use input_color_debug::DlssRayReconstructionInputColorDebug;

use bevy_app::{App, Plugin};
use bevy_camera::Hdr;
use bevy_core_pipeline::{
    prepass::{DepthPrepass, MotionVectorPrepass},
    schedule::{Core3d, Core3dSystems},
};
use bevy_ecs::prelude::*;
use bevy_math::{UVec2, Vec2};
use bevy_reflect::{reflect_remote, Reflect};
use bevy_render::{
    camera::{MipBias, TemporalJitter},
    renderer::{
        raw_vulkan_init::{AdditionalVulkanFeatures, RawVulkanInitSettings},
        RenderDevice, RenderQueue,
    },
    texture::CachedTexture,
    view::prepare_view_targets,
    ExtractSchedule, Render, RenderApp, RenderSystems,
};
use dlss_wgpu::{
    ray_reconstruction::{
        DlssRayReconstruction, DlssRayReconstructionDepthMode, DlssRayReconstructionRoughnessMode,
    },
    super_resolution::DlssSuperResolution,
    FeatureSupport,
};
use std::{
    marker::PhantomData,
    ops::Deref,
    sync::{Arc, Mutex},
};
use tracing::warn; // 保留各个DLSS初始化阶段的实际错误，避免只报告泛化不支持。
use uuid::Uuid;

/// Initializes DLSS support in the renderer. This must be registered before [`RenderPlugin`](bevy_render::RenderPlugin) because
/// it configures render init code.
#[derive(Default)]
pub struct DlssInitPlugin;

impl Plugin for DlssInitPlugin {
    #[allow(unsafe_code)]
    fn build(&self, app: &mut App) {
        let dlss_project_id = app.world().get_resource::<DlssProjectId>()
                        .expect("The `dlss` feature is enabled, but DlssProjectId was not added to the App before DlssInitPlugin.").0;
        let mut raw_vulkan_settings = app
            .world_mut()
            .get_resource_or_init::<RawVulkanInitSettings>();

        // SAFETY: this does not remove any instance features and only enables features that are supported
        unsafe {
            raw_vulkan_settings.add_create_instance_callback(
                move |mut args, additional_vulkan_features| {
                    let mut feature_support = FeatureSupport::default();
                    match dlss_wgpu::register_instance_extensions(
                        dlss_project_id,
                        &mut args,
                        &mut feature_support,
                    ) {
                        Ok(_) => {
                            if feature_support.super_resolution_supported {
                                additional_vulkan_features.insert::<DlssSuperResolutionSupported>();
                            }
                            if feature_support.ray_reconstruction_supported {
                                additional_vulkan_features
                                    .insert::<DlssRayReconstructionSupported>();
                            }
                        }
                        Err(error) => warn!(?error, "DLSS instance extension registration failed"), // 保留官方NGX/扩展错误，不吞掉第一阶段失败。
                    }
                },
            );
        }

        // SAFETY: this does not remove any device features and only enables features that are supported
        unsafe {
            raw_vulkan_settings.add_create_device_callback(
                move |mut args, adapter, additional_vulkan_features| {
                    let mut feature_support = FeatureSupport::default();
                    match dlss_wgpu::register_device_extensions(
                        dlss_project_id,
                        &mut args,
                        adapter,
                        &mut feature_support,
                    ) {
                        Ok(_) => {
                            if feature_support.super_resolution_supported {
                                additional_vulkan_features.insert::<DlssSuperResolutionSupported>();
                            } else {
                                additional_vulkan_features.remove::<DlssSuperResolutionSupported>();
                            }
                            if feature_support.ray_reconstruction_supported {
                                additional_vulkan_features
                                    .insert::<DlssRayReconstructionSupported>();
                            } else {
                                additional_vulkan_features
                                    .remove::<DlssRayReconstructionSupported>();
                            }
                        }
                        Err(error) => warn!(?error, "DLSS device extension registration failed"), // 区分设备扩展失败与后续SDK初始化失败。
                    }
                },
            )
        };
    }
}

/// Enables DLSS support. This requires [`DlssInitPlugin`] to function, which must be manually registered in the correct order
/// prior to registering this plugin.
#[derive(Default)]
pub struct DlssPlugin;

impl Plugin for DlssPlugin {
    fn build(&self, app: &mut App) {
        input_color_debug::build(app);
        app.register_type::<Dlss<DlssSuperResolutionFeature>>()
            .register_type::<Dlss<DlssRayReconstructionFeature>>();
    }

    fn finish(&self, app: &mut App) {
        let (super_resolution_supported, ray_reconstruction_supported) = {
            let features = app
                .sub_app_mut(RenderApp)
                .world()
                .resource::<AdditionalVulkanFeatures>();
            (
                features.has::<DlssSuperResolutionSupported>(),
                features.has::<DlssRayReconstructionSupported>(),
            )
        };
        let telemetry = DlssRuntimeTelemetry::default();
        app.insert_resource(telemetry.clone()); // 主世界设置状态读取 SDK 实际内部尺寸与非致命失败。
        app.sub_app_mut(RenderApp).insert_resource(telemetry); // RenderApp 通过共享快照回传上下文结果。
        if !super_resolution_supported {
            warn!(super_resolution_supported, ray_reconstruction_supported, "DLSS SDK initialization skipped: Vulkan capability flags unavailable"); // 明确尚未进入NGX初始化，避免误判搜索路径。
            return;
        }

        let wgpu_device = {
            let render_world = app.sub_app(RenderApp).world();
            let render_device = render_world.resource::<RenderDevice>().wgpu_device();
            render_device.clone()
        };
        let project_id = app.world().get_resource::<DlssProjectId>()
            .expect("The `dlss` feature is enabled, but DlssProjectId was not added to the App before DlssPlugin.");
        let dlss_sdk = match dlss_wgpu::DlssSdk::new(project_id.0, wgpu_device) { // 成功值直接进入资源，不使用检查后unwrap。
            Ok(sdk) => sdk, // 保持成功初始化对象所有权。
            Err(error) => { // SDK错误必须与扩展注册阶段分开记录。
                warn!(?error, "DLSS SDK initialization failed"); // 保留真实DlssError供SR与Streamline共存排障。
                return; // 继续现有安全回退，不伪造支持状态。
            } // 初始化错误处理结束。
        }; // SDK初始化结果处理结束。

        app.insert_resource(DlssSuperResolutionSupported);
        if ray_reconstruction_supported {
            app.insert_resource(DlssRayReconstructionSupported);
        }

        app.sub_app_mut(RenderApp)
            .insert_resource(DlssSdk(dlss_sdk)) // 使用已经匹配成功的SDK对象。
            .add_systems(
                ExtractSchedule,
                (
                    extract::extract_dlss::<DlssSuperResolutionFeature>,
                    extract::extract_dlss::<DlssRayReconstructionFeature>,
                ),
            )
            .add_systems(
                Render,
                (
                    prepare::prepare_dlss::<DlssSuperResolutionFeature>,
                    prepare::prepare_dlss::<DlssRayReconstructionFeature>,
                )
                    .in_set(RenderSystems::PrepareViews)
                    .before(prepare_view_targets),
            );

        input_color_debug::finish(app.sub_app_mut(RenderApp));
        app.sub_app_mut(RenderApp).add_systems(
            Core3d,
            (
                node::dlss_super_resolution,
                node::dlss_ray_reconstruction,
                input_color_debug::show_input_color,
            )
                .chain()
                .in_set(Core3dSystems::EarlyPostProcess),
        );
    }
}

/// Camera component to enable DLSS.
#[derive(Component, Reflect, Clone)]
#[reflect(Component)]
#[require(TemporalJitter, MipBias, DepthPrepass, MotionVectorPrepass, Hdr)]
pub struct Dlss<F: DlssFeature = DlssSuperResolutionFeature> {
    /// How much upscaling should be applied.
    #[reflect(remote = DlssPerfQualityModeRemoteReflect)]
    pub perf_quality_mode: DlssPerfQualityMode,
    /// Set to true to delete the saved temporal history (past frames).
    ///
    /// Useful for preventing ghosting when the history is no longer
    /// representative of the current frame, such as in sudden camera cuts.
    ///
    /// After setting this to true, it will automatically be toggled
    /// back to false at the end of the frame.
    pub reset: bool,
    #[reflect(ignore)]
    pub _phantom_data: PhantomData<F>,
}

impl<F: DlssFeature> Default for Dlss<F> {
    fn default() -> Self {
        Self {
            perf_quality_mode: Default::default(),
            reset: Default::default(),
            _phantom_data: Default::default(),
        }
    }
}

pub trait DlssFeature: Reflect + Clone + Default {
    type Context: Send;

    /// 标识遥测写入 SR 或 RR 独立槽位，防止两种组件交接时互相覆盖结果。
    fn telemetry_feature() -> DlssTelemetryFeature;

    fn upscaled_resolution(context: &Self::Context) -> UVec2;

    fn render_resolution(context: &Self::Context) -> UVec2;

    fn suggested_jitter(
        context: &Self::Context,
        frame_number: u32,
        render_resolution: UVec2,
    ) -> Vec2;

    fn suggested_mip_bias(context: &Self::Context, render_resolution: UVec2) -> f32;

    fn new_context(
        upscaled_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        feature_flags: dlss_wgpu::DlssFeatureFlags,
        sdk: Arc<Mutex<dlss_wgpu::DlssSdk>>,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<Self::Context, dlss_wgpu::DlssError>;
}

/// DLSS Super Resolution.
///
/// Only available when the [`DlssSuperResolutionSupported`] resource exists.
#[derive(Reflect, Clone, Default)]
pub struct DlssSuperResolutionFeature;

impl DlssFeature for DlssSuperResolutionFeature {
    type Context = DlssSuperResolution;

    fn telemetry_feature() -> DlssTelemetryFeature {
        DlssTelemetryFeature::SuperResolution
    }

    fn upscaled_resolution(context: &Self::Context) -> UVec2 {
        context.upscaled_resolution().into()
    }

    fn render_resolution(context: &Self::Context) -> UVec2 {
        context.render_resolution().into()
    }

    fn suggested_jitter(
        context: &Self::Context,
        frame_number: u32,
        render_resolution: UVec2,
    ) -> Vec2 {
        context
            .suggested_jitter(frame_number, render_resolution.to_array())
            .into()
    }

    fn suggested_mip_bias(context: &Self::Context, render_resolution: UVec2) -> f32 {
        context.suggested_mip_bias(render_resolution.to_array())
    }

    fn new_context(
        upscaled_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        feature_flags: dlss_wgpu::DlssFeatureFlags,
        sdk: Arc<Mutex<dlss_wgpu::DlssSdk>>,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<Self::Context, dlss_wgpu::DlssError> {
        DlssSuperResolution::new(
            upscaled_resolution.to_array(),
            perf_quality_mode,
            feature_flags,
            sdk,
            device.wgpu_device(),
            queue.deref(),
        )
    }
}

/// DLSS Ray Reconstruction.
///
/// Only available when the [`DlssRayReconstructionSupported`] resource exists.
#[derive(Reflect, Clone, Default)]
pub struct DlssRayReconstructionFeature;

impl DlssFeature for DlssRayReconstructionFeature {
    type Context = DlssRayReconstruction;

    fn telemetry_feature() -> DlssTelemetryFeature {
        DlssTelemetryFeature::RayReconstruction
    }

    fn upscaled_resolution(context: &Self::Context) -> UVec2 {
        context.upscaled_resolution().into()
    }

    fn render_resolution(context: &Self::Context) -> UVec2 {
        context.render_resolution().into()
    }

    fn suggested_jitter(
        context: &Self::Context,
        frame_number: u32,
        render_resolution: UVec2,
    ) -> Vec2 {
        context
            .suggested_jitter(frame_number, render_resolution.to_array())
            .into()
    }

    fn suggested_mip_bias(context: &Self::Context, render_resolution: UVec2) -> f32 {
        context.suggested_mip_bias(render_resolution.to_array())
    }

    fn new_context(
        upscaled_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        feature_flags: dlss_wgpu::DlssFeatureFlags,
        sdk: Arc<Mutex<dlss_wgpu::DlssSdk>>,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<Self::Context, dlss_wgpu::DlssError> {
        DlssRayReconstruction::new(
            upscaled_resolution.to_array(),
            perf_quality_mode,
            // Not supported by ray reconstruction
            feature_flags - dlss_wgpu::DlssFeatureFlags::AutoExposure,
            DlssRayReconstructionRoughnessMode::Packed,
            DlssRayReconstructionDepthMode::Hardware,
            sdk,
            device.wgpu_device(),
            queue.deref(),
        )
    }
}

/// Additional textures needed as inputs for [`DlssRayReconstructionFeature`].
#[derive(Component)]
pub struct ViewDlssRayReconstructionTextures {
    pub diffuse_albedo: CachedTexture,
    pub specular_albedo: CachedTexture,
    pub normal_roughness: CachedTexture,
    pub depth: CachedTexture,
    pub specular_motion_vectors: CachedTexture,
}

#[reflect_remote(DlssPerfQualityMode)]
#[derive(Default)]
enum DlssPerfQualityModeRemoteReflect {
    #[default]
    Auto,
    Dlaa,
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

#[derive(Resource)]
struct DlssSdk(Arc<Mutex<dlss_wgpu::DlssSdk>>);

/// RenderApp 遥测槽位；SR 与 RR 在原子交接期间必须保持彼此独立。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DlssTelemetryFeature {
    SuperResolution,
    RayReconstruction,
}

/// 最近一次指定 DLSS 特性创建上下文的实际尺寸与失败结果。
#[derive(Clone, Debug, Default)]
pub struct DlssRuntimeTelemetryFeatureStatus {
    pub output_resolution: UVec2,
    pub internal_resolution: UVec2,
    pub perf_quality_mode: Option<DlssPerfQualityMode>,
    pub failure: Option<String>,
}

/// 主世界可读取的 SR/RR 独立遥测快照。
#[derive(Clone, Debug, Default)]
pub struct DlssRuntimeTelemetrySnapshot {
    pub super_resolution: DlssRuntimeTelemetryFeatureStatus,
    pub ray_reconstruction: DlssRuntimeTelemetryFeatureStatus,
}

/// RenderApp 与主世界共享的低频 DLSS 上下文结果；只在创建成功或失败时写入。
#[derive(Resource, Clone, Default)]
pub struct DlssRuntimeTelemetry(Arc<Mutex<DlssRuntimeTelemetrySnapshot>>);

impl DlssRuntimeTelemetry {
    /// 读取一致快照；互斥锁异常时安全返回空状态而不是影响渲染。
    pub fn snapshot(&self) -> DlssRuntimeTelemetrySnapshot {
        self.0.lock().map(|snapshot| snapshot.clone()).unwrap_or_default()
    }

    fn record_success(
        &self,
        feature: DlssTelemetryFeature,
        output_resolution: UVec2,
        internal_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
    ) {
        let Ok(mut snapshot) = self.0.lock() else {
            return;
        };
        let target = match feature {
            DlssTelemetryFeature::SuperResolution => &mut snapshot.super_resolution,
            DlssTelemetryFeature::RayReconstruction => &mut snapshot.ray_reconstruction,
        };
        *target = DlssRuntimeTelemetryFeatureStatus {
            output_resolution,
            internal_resolution,
            perf_quality_mode: Some(perf_quality_mode),
            failure: None,
        };
    }

    fn record_failure(
        &self,
        feature: DlssTelemetryFeature,
        output_resolution: UVec2,
        perf_quality_mode: DlssPerfQualityMode,
        failure: String,
    ) {
        let Ok(mut snapshot) = self.0.lock() else {
            return;
        };
        let target = match feature {
            DlssTelemetryFeature::SuperResolution => &mut snapshot.super_resolution,
            DlssTelemetryFeature::RayReconstruction => &mut snapshot.ray_reconstruction,
        };
        *target = DlssRuntimeTelemetryFeatureStatus {
            output_resolution,
            internal_resolution: UVec2::ZERO,
            perf_quality_mode: Some(perf_quality_mode),
            failure: Some(failure),
        };
    }
}

/// Application-specific ID for DLSS.
///
/// See the DLSS programming guide for more info.
#[derive(Resource, Clone)]
pub struct DlssProjectId(pub Uuid);

/// When DLSS Super Resolution is supported by the current system, this resource will exist in the main world.
/// Otherwise this resource will be absent.
#[derive(Resource, Clone, Copy)]
pub struct DlssSuperResolutionSupported;

/// When DLSS Ray Reconstruction is supported by the current system, this resource will exist in the main world.
/// Otherwise this resource will be absent.
#[derive(Resource, Clone, Copy)]
pub struct DlssRayReconstructionSupported;
