use super::{Dlss, DlssFeature, DlssRuntimeTelemetry, DlssSdk};
use bevy_camera::{Camera3d, CameraMainTextureUsages, MainPassResolutionOverride};
use bevy_core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass};
use bevy_diagnostic::FrameCount;
use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::With,
    system::{Commands, Query, Res},
};
use bevy_math::Vec4Swizzles;
use bevy_render::{
    camera::{MipBias, TemporalJitter},
    render_resource::TextureUsages,
    renderer::{RenderDevice, RenderQueue},
    view::ExtractedView,
};
use dlss_wgpu::{DlssFeatureFlags, DlssPerfQualityMode};
use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};
use tracing::warn;

#[derive(Component)]
pub struct DlssRenderContext<F: DlssFeature> {
    pub context: Mutex<F::Context>,
    pub perf_quality_mode: DlssPerfQualityMode,
    pub feature_flags: DlssFeatureFlags,
}

/// 记住同一输出尺寸与模式的上下文创建或求值失败，避免稳定帧重复尝试或日志刷屏。
#[derive(Component)]
pub struct DlssRuntimeFailure<F: DlssFeature> {
    pub upscaled_resolution: bevy_math::UVec2,
    pub perf_quality_mode: DlssPerfQualityMode,
    pub feature_flags: DlssFeatureFlags,
    pub marker: PhantomData<F>,
}

pub fn prepare_dlss<F: DlssFeature>(
    mut query: Query<
        (
            Entity,
            &ExtractedView,
            &Dlss<F>,
            &mut Camera3d,
            &mut CameraMainTextureUsages,
            &mut TemporalJitter,
            &mut MipBias,
            Option<&mut DlssRenderContext<F>>,
            Option<&DlssRuntimeFailure<F>>,
        ),
        (
            With<Camera3d>,
            With<TemporalJitter>,
            With<DepthPrepass>,
            With<MotionVectorPrepass>,
        ),
    >,
    dlss_sdk: Res<DlssSdk>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    telemetry: Res<DlssRuntimeTelemetry>,
    frame_count: Res<FrameCount>,
    mut commands: Commands,
) {
    for (
        entity,
        view,
        dlss,
        mut camera_3d,
        mut camera_main_texture_usages,
        mut temporal_jitter,
        mut mip_bias,
        mut dlss_context,
        context_failure,
    ) in &mut query
    {
        // NGX既以存储图像写入主颜色，也会通过clear_texture清空输出，因此两种用途缺一不可。
        camera_main_texture_usages.0 |=
            TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST;

        let mut depth_texture_usages = TextureUsages::from(camera_3d.depth_texture_usages);
        depth_texture_usages |= TextureUsages::TEXTURE_BINDING;
        camera_3d.depth_texture_usages = depth_texture_usages.into();

        let upscaled_resolution = view.viewport.zw();

        let dlss_feature_flags = DlssFeatureFlags::LowResolutionMotionVectors
            | DlssFeatureFlags::InvertedDepth
            | DlssFeatureFlags::HighDynamicRange
            | DlssFeatureFlags::AutoExposure; // TODO

        if context_failure.is_some_and(|failure| {
            failure.upscaled_resolution == upscaled_resolution
                && failure.perf_quality_mode == dlss.perf_quality_mode
                && failure.feature_flags == dlss_feature_flags
        }) {
            continue; // 同一不支持组合保持安全回退；尺寸或模式变化会自动重新查询。
        }

        match dlss_context.as_deref_mut() {
            Some(dlss_context)
                if upscaled_resolution
                    == F::upscaled_resolution(&dlss_context.context.lock().unwrap())
                    && dlss.perf_quality_mode == dlss_context.perf_quality_mode
                    && dlss_feature_flags == dlss_context.feature_flags =>
            {
                let dlss_context = dlss_context.context.lock().unwrap();
                let render_resolution = F::render_resolution(&dlss_context);
                temporal_jitter.offset =
                    F::suggested_jitter(&dlss_context, frame_count.0, render_resolution);
            }
            _ => {
                let dlss_context = match F::new_context(
                    upscaled_resolution,
                    dlss.perf_quality_mode,
                    dlss_feature_flags,
                    Arc::clone(&dlss_sdk.0),
                    &render_device,
                    &render_queue,
                ) {
                    Ok(context) => context,
                    Err(error) => {
                        let failure = error.to_string();
                        warn!(
                            "DLSS context creation failed for {} at {}x{} in {:?}: {error}",
                            std::any::type_name::<F>(),
                            upscaled_resolution.x,
                            upscaled_resolution.y,
                            dlss.perf_quality_mode,
                        );
                        commands
                            .entity(entity)
                            .remove::<(DlssRenderContext<F>, MainPassResolutionOverride)>()
                            .insert(DlssRuntimeFailure::<F> {
                                upscaled_resolution,
                                perf_quality_mode: dlss.perf_quality_mode,
                                feature_flags: dlss_feature_flags,
                                marker: PhantomData,
                            });
                        telemetry.record_failure(
                            F::telemetry_feature(),
                            upscaled_resolution,
                            dlss.perf_quality_mode,
                            failure,
                        );
                        continue; // 不支持模式或运行库失败不得让 RenderApp panic。
                    }
                };

                let render_resolution = F::render_resolution(&dlss_context);
                temporal_jitter.offset =
                    F::suggested_jitter(&dlss_context, frame_count.0, render_resolution);
                mip_bias.0 = F::suggested_mip_bias(&dlss_context, render_resolution);
                telemetry.record_success(
                    F::telemetry_feature(),
                    upscaled_resolution,
                    render_resolution,
                    dlss.perf_quality_mode,
                );

                commands.entity(entity).insert((
                    DlssRenderContext::<F> {
                        context: Mutex::new(dlss_context),
                        perf_quality_mode: dlss.perf_quality_mode,
                        feature_flags: dlss_feature_flags,
                    },
                    MainPassResolutionOverride(render_resolution),
                ));
                commands
                    .entity(entity)
                    .remove::<DlssRuntimeFailure<F>>(); // 模式/尺寸变化后成功恢复时清除旧失败状态。
            }
        }
    }
}
