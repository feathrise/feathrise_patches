use super::{
    prepare::{DlssRenderContext, DlssRuntimeFailure},
    Dlss, DlssRayReconstructionFeature, DlssRuntimeTelemetry, DlssSuperResolutionFeature,
    DlssTelemetryFeature, ViewDlssRayReconstructionTextures,
};
use bevy_camera::MainPassResolutionOverride;
use bevy_core_pipeline::prepass::ViewPrepassTextures;
use bevy_ecs::system::{Commands, Res};
use bevy_math::UVec2;
use bevy_render::{
    camera::TemporalJitter,
    diagnostic::RecordDiagnostics,
    renderer::{RenderAdapter, RenderContext, ViewQuery},
    view::ViewTarget,
};
use std::marker::PhantomData;
use tracing::warn;
use dlss_wgpu::{
    ray_reconstruction::{
        DlssRayReconstructionRenderParameters, DlssRayReconstructionSpecularGuide,
    },
    super_resolution::{DlssSuperResolutionExposure, DlssSuperResolutionRenderParameters},
};

pub fn dlss_super_resolution(
    view: ViewQuery<(
        &Dlss<DlssSuperResolutionFeature>,
        &DlssRenderContext<DlssSuperResolutionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &ViewTarget,
        &ViewPrepassTextures,
    )>,
    adapter: Res<RenderAdapter>,
    telemetry: Res<DlssRuntimeTelemetry>,
    mut commands: Commands,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (dlss, dlss_context, resolution_override, temporal_jitter, view_target, prepass_textures) =
        view.into_inner();

    let (Some(prepass_depth_texture), Some(prepass_motion_vectors_texture)) =
        (&prepass_textures.depth, &prepass_textures.motion_vectors)
    else {
        return;
    };

    // 先借用当前/备用颜色视图，不立即翻转 Bevy 的后处理双缓冲。NGX 求值失败时必须
    // 让后续节点继续看到原主颜色，避免安全回退前出现一帧未写入的目标。
    let source = view_target.main_texture_view();
    let destination = view_target.main_texture_other_view();

    let render_resolution = resolution_override.0;
    let render_parameters = DlssSuperResolutionRenderParameters {
        color: source,
        depth: &prepass_depth_texture.texture.default_view,
        motion_vectors: &prepass_motion_vectors_texture.texture.default_view,
        exposure: DlssSuperResolutionExposure::Automatic, // TODO
        bias: None,                                       // TODO
        dlss_output: destination,
        reset: dlss.reset,
        jitter_offset: (-temporal_jitter.offset).to_array(),
        partial_texture_size: Some(render_resolution.to_array()),
        motion_vector_scale: Some((-render_resolution.as_vec2()).to_array()),
    };

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dlss_super_resolution");

    let feature_flags = dlss_context.feature_flags; // 运行失败后用原创建参数建立稳定阻断键。
    let mut dlss_context = dlss_context.context.lock().unwrap();
    let dlss_command_buffer = match dlss_context.render(
        render_parameters,
        ctx.command_encoder(),
        &adapter,
    ) {
        Ok(command_buffer) => command_buffer,
        Err(error) => {
            let output_resolution = UVec2::from_array(dlss_context.upscaled_resolution());
            let failure = error.to_string();
            telemetry.record_failure(
                DlssTelemetryFeature::SuperResolution,
                output_resolution,
                dlss.perf_quality_mode,
                failure.clone(),
            );
            commands
                .entity(entity)
                .remove::<(
                    DlssRenderContext<DlssSuperResolutionFeature>,
                    MainPassResolutionOverride,
                )>()
                .insert(DlssRuntimeFailure::<DlssSuperResolutionFeature> {
                    upscaled_resolution: output_resolution,
                    perf_quality_mode: dlss.perf_quality_mode,
                    feature_flags,
                    marker: PhantomData,
                });
            warn!("DLSS Super Resolution evaluation failed and fell back safely: {failure}");
            return; // 运行时 NGX 失败不得让 RenderApp panic；主世界下一帧归还普通画质租约。
        }
    };

    let _committed_write = view_target.post_process_write(); // 只有成功录制NGX命令才把备用纹理发布为新的主颜色。
    ctx.add_command_buffer(dlss_command_buffer);
    time_span.end(ctx.command_encoder());
}

pub fn dlss_ray_reconstruction(
    view: ViewQuery<(
        &Dlss<DlssRayReconstructionFeature>,
        &DlssRenderContext<DlssRayReconstructionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &ViewTarget,
        &ViewPrepassTextures,
        &ViewDlssRayReconstructionTextures,
    )>,
    adapter: Res<RenderAdapter>,
    telemetry: Res<DlssRuntimeTelemetry>,
    mut commands: Commands,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (
        dlss,
        dlss_context,
        resolution_override,
        temporal_jitter,
        view_target,
        prepass_textures,
        ray_reconstruction_textures,
    ) = view.into_inner();

    let (Some(prepass_depth_texture), Some(prepass_motion_vectors_texture)) =
        (&prepass_textures.depth, &prepass_textures.motion_vectors)
    else {
        return;
    };

    // RR 与 SR 使用同一提交协议：失败保持当前主颜色，成功后才原子发布备用纹理。
    let source = view_target.main_texture_view();
    let destination = view_target.main_texture_other_view();

    let render_resolution = resolution_override.0;
    let render_parameters = DlssRayReconstructionRenderParameters {
        diffuse_albedo: &ray_reconstruction_textures.diffuse_albedo.default_view,
        specular_albedo: &ray_reconstruction_textures.specular_albedo.default_view,
        normals: &ray_reconstruction_textures.normal_roughness.default_view,
        roughness: None,
        color: source,
        depth: &prepass_depth_texture.texture.default_view,
        motion_vectors: &prepass_motion_vectors_texture.texture.default_view,
        specular_guide: DlssRayReconstructionSpecularGuide::SpecularMotionVectors(
            &ray_reconstruction_textures
                .specular_motion_vectors
                .default_view,
        ),
        screen_space_subsurface_scattering_guide: None, // TODO
        bias: None,                                     // TODO
        dlss_output: destination,
        reset: dlss.reset,
        jitter_offset: (-temporal_jitter.offset).to_array(),
        partial_texture_size: Some(render_resolution.to_array()),
        motion_vector_scale: Some((-render_resolution.as_vec2()).to_array()),
    };

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dlss_ray_reconstruction");

    let feature_flags = dlss_context.feature_flags; // RR 的失败重试键同样必须保留原始创建标志。
    let mut dlss_context = dlss_context.context.lock().unwrap();
    let dlss_command_buffer = match dlss_context.render(
        render_parameters,
        ctx.command_encoder(),
        &adapter,
    ) {
        Ok(command_buffer) => command_buffer,
        Err(error) => {
            let output_resolution = UVec2::from_array(dlss_context.upscaled_resolution());
            let failure = error.to_string();
            telemetry.record_failure(
                DlssTelemetryFeature::RayReconstruction,
                output_resolution,
                dlss.perf_quality_mode,
                failure.clone(),
            );
            commands
                .entity(entity)
                .remove::<(
                    DlssRenderContext<DlssRayReconstructionFeature>,
                    MainPassResolutionOverride,
                )>()
                .insert(DlssRuntimeFailure::<DlssRayReconstructionFeature> {
                    upscaled_resolution: output_resolution,
                    perf_quality_mode: dlss.perf_quality_mode,
                    feature_flags,
                    marker: PhantomData,
                });
            warn!("DLSS Ray Reconstruction evaluation failed and fell back safely: {failure}");
            return; // RR 失败后保持 SR 暂停声明到主世界看到阻断，再按交接状态机安全恢复。
        }
    };

    let _committed_write = view_target.post_process_write(); // 后续后处理只会观察到已经由NGX命令写入的目标。
    ctx.add_command_buffer(dlss_command_buffer);
    time_span.end(ctx.command_encoder());
}
