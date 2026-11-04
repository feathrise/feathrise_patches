use crate::blit::{BlitPipeline, BlitPipelineKey};
use bevy_app::prelude::*;
use bevy_camera::CameraOutputMode;
use bevy_ecs::prelude::*;
use bevy_render::{
    camera::ExtractedCamera, render_resource::*, view::ViewTarget, Render, RenderApp,
    RenderStartup, RenderSystems,
};

mod node;
mod native_output; // 私有窗口编码判定，避免扩展公用BlitPipelineKey影响其他渲染域。

pub use node::upscaling;

pub struct UpscalingPlugin;

impl Plugin for UpscalingPlugin {
    fn build(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                Render,
                // This system should probably technically be run *after* all of the other systems
                // that might modify `PipelineCache` via interior mutability, but for now,
                // we've chosen to simply ignore the ambiguities out of a desire for a better refactor
                // and aversion to extensive and intrusive system ordering.
                // See https://github.com/bevyengine/bevy/issues/14770 for more context.
                prepare_view_upscaling_pipelines
                    .in_set(RenderSystems::Prepare)
                    .ambiguous_with_all(),
            );
            render_app.add_systems(RenderStartup, clear_view_upscaling_pipelines);
        }
    }
}

#[derive(Component)]
pub struct ViewUpscalingPipeline(CachedRenderPipelineId, BlitPipelineKey, bool); // 真实管线身份同时包含输出编码策略。

impl ViewUpscalingPipeline { // 保护系统必须检查真正绘制的管线，不能重新猜测普通blit键。
    pub fn is_ready(&self, cache: &PipelineCache) -> bool { cache.get_render_pipeline(self.0).is_some() } // 只查询已编译状态，不阻塞或排队。
    pub fn encodes_srgb(&self) -> bool { self.2 } // 清帧回退也必须与实际UNORM输出编码保持一致。
} // 视图管线公开只读查询结束。

/// This is not required on first startup but is required during render recovery
fn clear_view_upscaling_pipelines(
    mut commands: Commands,
    views: Query<Entity, With<ViewUpscalingPipeline>>,
) {
    for entity in &views {
        commands.entity(entity).remove::<ViewUpscalingPipeline>();
    }
}

fn prepare_view_upscaling_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>, // 管线编译通过内部队列异步推进。
    mut pipelines: ResMut<SpecializedRenderPipelines<BlitPipeline>>,
    blit_pipeline: Res<BlitPipeline>,
    view_targets: Query<(
        Entity,
        &ViewTarget,
        Option<&ExtractedCamera>,
        Option<&ViewUpscalingPipeline>,
    )>,
) {
    for (entity, view_target, camera, maybe_pipeline) in view_targets.iter() {
        let blend_state = if let Some(extracted_camera) = camera {
            match extracted_camera.output_mode {
                CameraOutputMode::Skip => None,
                CameraOutputMode::Write { blend_state, .. } => {
                    match blend_state {
                        None => {
                            // Auto-detect: the first camera to render to this output
                            // (sorted_camera_index_for_target == 0) uses replace mode;
                            // subsequent cameras default to alpha blending so they don't
                            // accidentally overwrite earlier cameras' output.
                            if extracted_camera.sorted_camera_index_for_target > 0 {
                                Some(BlendState::ALPHA_BLENDING)
                            } else {
                                None
                            }
                        }
                        _ => blend_state,
                    }
                }
            }
        } else {
            None
        };

        let Some(target_format) = view_target.out_texture_view_format() else {
            continue;
        };

        let key = BlitPipelineKey {
            target_format,
            blend_state,
            samples: 1,
            source_space: view_target.compositing_space,
        };

        let encode_srgb = native_output::requires_encoding(camera, target_format); // 只对真实窗口的8位UNORM输出编码；纹理目标不变。
        if maybe_pipeline.is_none_or(|ViewUpscalingPipeline(_, cached_key, cached_encoding)| *cached_key != key || *cached_encoding != encode_srgb) { // 视图规格不变时不重复排队。
            let pipeline = if encode_srgb { // 私有变体不污染公用blit缓存或键。
                let mut descriptor = blit_pipeline.specialize(key); // 复用原始布局、混合、采样和源空间转换。
                descriptor.fragment.as_mut().expect("blit has fragment shader").shader_defs.push("LINEAR_TO_SRGB".into()); // 最后补一次线性到sRGB编码。
                pipeline_cache.queue_render_pipeline(descriptor) // 异步编译；未就绪时由节点初始化当前输出。
            } else { pipelines.specialize(&pipeline_cache, &blit_pipeline, key) }; // 普通窗口及纹理目标保留原有专用化缓存。

            commands
                .entity(entity)
                .insert(ViewUpscalingPipeline(pipeline, key, encode_srgb)); // 原子更新管线与其完整输出身份。
        }
    }
}
