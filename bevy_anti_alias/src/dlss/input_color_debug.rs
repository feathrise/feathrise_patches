//! Debug presentation of the same-frame color input consumed by DLSS Ray Reconstruction.

use super::{
    prepare::DlssRenderContext, Dlss, DlssRayReconstructionFeature,
    ViewDlssRayReconstructionTextures,
};
use bevy_app::{App, SubApp};
use bevy_asset::{embedded_asset, load_embedded_asset, AssetServer};
use bevy_camera::{Camera, MainPassResolutionOverride};
use bevy_core_pipeline::{prepass::ViewPrepassTextures, FullscreenShader};
use bevy_ecs::prelude::*;
use bevy_math::UVec2;
use bevy_render::{
    camera::TemporalJitter,
    diagnostic::RecordDiagnostics,
    extract_component::{ExtractComponent, ExtractComponentPlugin},
    render_resource::{
        binding_types::{texture_2d, uniform_buffer},
        BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
        CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, Operations,
        PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
        ShaderStages, ShaderType, TextureFormat, TextureSampleType, TextureViewId, UniformBuffer,
    },
    renderer::{RenderContext, RenderQueue, ViewQuery},
    view::ViewTarget,
    RenderApp, RenderStartup,
};
use bevy_utils::default;

/// Presents the same-frame HDR color input consumed by DLSS Ray Reconstruction.
///
/// Ray Reconstruction still executes normally. After it completes, the debug pass overwrites its
/// current output with a nearest-neighbor presentation of the untouched input texture. This keeps
/// jitter, guides, history, and the selected performance/quality mode unchanged for an isolated
/// before/after comparison.
#[derive(Clone, Copy, Component, ExtractComponent)]
#[extract_app(RenderApp)]
#[extract_component_filter(With<Camera>)]
pub struct DlssRayReconstructionInputColorDebug;

/// Registers extraction while the main application still owns the camera component.
pub(super) fn build(app: &mut App) {
    embedded_asset!(app, "input_color_debug.wesl");
    app.add_plugins(ExtractComponentPlugin::<DlssRayReconstructionInputColorDebug>::default());
}

/// Creates the fixed HDR pipeline before any gameplay camera can request the debug view.
pub(super) fn finish(render_app: &mut SubApp) {
    render_app.add_systems(RenderStartup, init_input_color_debug_pipeline);
}

#[derive(Resource)]
pub(super) struct InputColorDebugPipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedRenderPipelineId,
}

#[derive(Clone, Copy, ShaderType)]
struct InputColorDebugUniform {
    input_extent: UVec2,
}

fn init_input_color_debug_pipeline(
    mut commands: Commands,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: ResMut<PipelineCache>,
) {
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "dlss_ray_reconstruction_input_color_debug_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                uniform_buffer::<InputColorDebugUniform>(false),
            ),
        ),
    );
    let pipeline_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("dlss_ray_reconstruction_input_color_debug".into()),
        layout: vec![bind_group_layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(asset_server.as_ref(), "input_color_debug.wesl"),
            targets: vec![Some(ColorTargetState {
                // `Dlss` requires `Hdr`, whose main target is RGBA16F.
                format: TextureFormat::Rgba16Float,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(InputColorDebugPipeline {
        bind_group_layout,
        pipeline_id,
    });
}

pub(super) struct CachedInputColorBindGroup {
    source_id: TextureViewId,
    input_extent: UVec2,
    bind_group: BindGroup,
    // The bind group retains the GPU buffer; this field also keeps its typed allocation explicit.
    _uniform: UniformBuffer<InputColorDebugUniform>,
}

/// Overwrites the RR result with its untouched same-frame color input without flipping ViewTarget.
pub(super) fn show_input_color(
    view: ViewQuery<(
        &Dlss<DlssRayReconstructionFeature>,
        &DlssRenderContext<DlssRayReconstructionFeature>,
        &MainPassResolutionOverride,
        &TemporalJitter,
        &ViewTarget,
        &ViewPrepassTextures,
        &ViewDlssRayReconstructionTextures,
        &DlssRayReconstructionInputColorDebug,
    )>,
    debug_pipeline: Res<InputColorDebugPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_queue: Res<RenderQueue>,
    mut ctx: RenderContext,
    mut cached_bind_group: Local<Option<CachedInputColorBindGroup>>,
) {
    let (
        _dlss,
        _dlss_context,
        resolution_override,
        _temporal_jitter,
        view_target,
        prepass_textures,
        _ray_reconstruction_textures,
        _debug,
    ) = view.into_inner();

    // Match the RR node's only early return so the alternate texture is known to be its input.
    if prepass_textures.motion_vectors.is_none() {
        return;
    }
    let Some(pipeline) = pipeline_cache.get_render_pipeline(debug_pipeline.pipeline_id) else {
        return;
    };
    if view_target.main_texture_format() != TextureFormat::Rgba16Float {
        return;
    }

    let output_size = view_target.main_texture().size();
    let output_extent = UVec2::new(output_size.width, output_size.height);
    let input_extent = resolution_override.0.min(output_extent);
    if input_extent.x == 0 || input_extent.y == 0 {
        return;
    }

    // RR has just flipped the target: current is its output, other is the untouched pInColor.
    let source = view_target.main_texture_other_view();
    let destination = view_target.main_texture_view();
    let needs_rebuild = cached_bind_group.as_ref().is_none_or(|cached| {
        cached.source_id != source.id() || cached.input_extent != input_extent
    });
    if needs_rebuild {
        let mut uniform = UniformBuffer::from(InputColorDebugUniform { input_extent });
        uniform.write_buffer(ctx.render_device(), &render_queue);
        let bind_group = ctx.render_device().create_bind_group(
            "dlss_ray_reconstruction_input_color_debug_bind_group",
            &pipeline_cache.get_bind_group_layout(&debug_pipeline.bind_group_layout),
            &BindGroupEntries::sequential((source, &uniform)),
        );
        *cached_bind_group = Some(CachedInputColorBindGroup {
            source_id: source.id(),
            input_extent,
            bind_group,
            _uniform: uniform,
        });
    }
    let bind_group = &cached_bind_group
        .as_ref()
        .expect("input color debug bind group was initialized")
        .bind_group;

    let pass_descriptor = RenderPassDescriptor {
        label: Some("dlss_ray_reconstruction_input_color_debug"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(
        ctx.command_encoder(),
        "dlss_ray_reconstruction_input_color_debug",
    );
    {
        let mut render_pass = ctx.command_encoder().begin_render_pass(&pass_descriptor);
        let pass_span = diagnostics.pass_span(
            &mut render_pass,
            "dlss_ray_reconstruction_input_color_debug",
        );
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        pass_span.end(&mut render_pass);
    }
    time_span.end(ctx.command_encoder());
}
