use crate::{fxaa::fxaa, smaa::smaa};
use bevy_app::prelude::*;
use bevy_asset::{embedded_asset, load_embedded_asset, AssetServer};
use bevy_camera::Camera;
use bevy_core_pipeline::{
    schedule::{Core2d, Core2dSystems, Core3d, Core3dSystems},
    FullscreenShader,
};
use bevy_ecs::{prelude::*, query::QueryItem};
use bevy_reflect::{std_traits::ReflectDefault, Reflect};
use bevy_render::{
    extract_component::{ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin},
    render_resource::{
        binding_types::{sampler, texture_2d, uniform_buffer},
        *,
    },
    renderer::RenderDevice,
    sync_component::SyncComponent,
    view::ExtractedView,
    Render, RenderApp, RenderStartup, RenderSystems,
};

mod node;

pub use node::cas;

/// Applies a contrast adaptive sharpening (CAS) filter to the camera.
///
/// CAS is usually used in combination with shader based anti-aliasing methods
/// such as FXAA or TAA to regain some of the lost detail from the blurring that they introduce.
///
/// CAS is designed to adjust the amount of sharpening applied to different areas of an image
/// based on the local contrast. This can help avoid over-sharpening areas with high contrast
/// and under-sharpening areas with low contrast.
///
/// To use this, add the [`ContrastAdaptiveSharpening`] component to a 2D or 3D camera.
#[derive(Component, Reflect, Clone)]
#[reflect(Component, Default, Clone)]
pub struct ContrastAdaptiveSharpening {
    /// Enable or disable sharpening.
    pub enabled: bool,
    /// Adjusts sharpening strength. Higher values increase the amount of sharpening.
    ///
    /// Clamped between 0.0 and 1.0.
    ///
    /// The default value is 0.6.
    pub sharpening_strength: f32,
    /// Whether to try and avoid sharpening areas that are already noisy.
    ///
    /// You probably shouldn't use this, and just leave it set to false.
    /// You should generally apply any sort of film grain or similar effects after CAS
    /// and upscaling to avoid artifacts.
    pub denoise: bool,
}

impl Default for ContrastAdaptiveSharpening {
    fn default() -> Self {
        ContrastAdaptiveSharpening {
            enabled: true,
            sharpening_strength: 0.6,
            denoise: false,
        }
    }
}

#[derive(Component, Default, Reflect, Clone)]
#[reflect(Component, Default, Clone)]
pub struct DenoiseCas(bool);

/// The uniform struct extracted from [`ContrastAdaptiveSharpening`] attached to a [`Camera`].
/// Will be available for use in the CAS shader.
#[doc(hidden)]
#[derive(Component, ShaderType, Clone)]
pub struct CasUniform {
    sharpness: f32,
}

impl SyncComponent<RenderApp> for ContrastAdaptiveSharpening {
    type Target = (DenoiseCas, CasUniform);
}

impl ExtractComponent<RenderApp> for ContrastAdaptiveSharpening {
    type QueryData = &'static Self;
    type QueryFilter = With<Camera>;
    type Out = (DenoiseCas, CasUniform);

    fn extract_component(item: QueryItem<Self::QueryData>) -> Option<Self::Out> {
        if !item.enabled || item.sharpening_strength == 0.0 {
            return None;
        }
        Some((
            DenoiseCas(item.denoise),
            CasUniform {
                // above 1.0 causes extreme artifacts and fireflies
                sharpness: item.sharpening_strength.clamp(0.0, 1.0),
            },
        ))
    }
}

/// Adds Support for Contrast Adaptive Sharpening (CAS).
#[derive(Default)]
pub struct CasPlugin;

impl Plugin for CasPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "robust_contrast_adaptive_sharpening.wesl");

        app.add_plugins((
            ExtractComponentPlugin::<ContrastAdaptiveSharpening>::default(),
            UniformComponentPlugin::<CasUniform>::default(),
        ));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_cas_pipeline)
            .add_systems(Render, prepare_cas_pipelines.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                cas.after(fxaa)
                    .after(smaa)
                    .in_set(Core3dSystems::PostProcess),
            )
            .add_systems(
                Core2d,
                cas.after(fxaa)
                    .after(smaa)
                    .in_set(Core2dSystems::PostProcess),
            );
    }
}

#[derive(Resource)]
pub struct CasPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    variants: Variants<RenderPipeline, CasPipelineSpecializer>,
}

pub fn init_cas_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "sharpening_texture_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                // CAS Settings
                uniform_buffer::<CasUniform>(true),
            ),
        ),
    );

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let fragment_shader = load_embedded_asset!(
        asset_server.as_ref(),
        "robust_contrast_adaptive_sharpening.wesl"
    );

    let variants = Variants::new(
        CasPipelineSpecializer,
        RenderPipelineDescriptor {
            label: Some("contrast_adaptive_sharpening".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: fragment_shader,
                ..Default::default()
            }),
            ..Default::default()
        },
    );

    commands.insert_resource(CasPipeline {
        layout,
        sampler,
        variants,
    });
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, SpecializerKey)]
pub struct CasPipelineKey {
    target_format: TextureFormat,
    denoise: bool,
}

pub struct CasPipelineSpecializer;

impl Specializer<RenderPipeline> for CasPipelineSpecializer {
    type Key = CasPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut <RenderPipeline as Specializable>::Descriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        let fragment = descriptor.fragment_mut()?;

        if key.denoise {
            fragment.shader_defs.push("RCAS_DENOISE".into());
        }

        fragment.set_target(
            0,
            ColorTargetState {
                format: key.target_format,
                blend: None,
                write_mask: ColorWrites::ALL,
            },
        );

        Ok(key)
    }
}

fn prepare_cas_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut sharpening_pipeline: ResMut<CasPipeline>,
    cameras: Query<
        (Entity, &ExtractedView, &DenoiseCas),
        Or<(Added<CasUniform>, Changed<DenoiseCas>)>,
    >,
    mut removals: RemovedComponents<CasUniform>,
) -> Result<(), BevyError> {
    for entity in removals.read() {
        remove_view_cas_pipeline(&mut commands, entity);
    }

    for (entity, view, denoise_cas) in &cameras {
        let pipeline_id = sharpening_pipeline.variants.specialize(
            &pipeline_cache,
            CasPipelineKey {
                denoise: denoise_cas.0,
                target_format: view.target_format,
            },
        )?;

        commands.entity(entity).insert(ViewCasPipeline(pipeline_id));
    }

    Ok(())
}

/// 清理 CAS 管线时允许 Render View 已先被同步世界销毁，避免延迟命令产生无效实体警告。
fn remove_view_cas_pipeline(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).try_remove::<ViewCasPipeline>(); // 活实体照常移除；陈旧实体在命令应用时静默跳过。
}

#[derive(Component)]
pub struct ViewCasPipeline(CachedRenderPipelineId);

#[cfg(test)]
mod tests {
    use super::*; // 直接测试生产清理 helper 与私有 CAS 管线组件。

    #[derive(Resource)]
    struct TestCasView(Entity); // 向测试系统提供待清理的 Render View 身份。

    fn remove_test_cas_pipeline(mut commands: Commands, view: Res<TestCasView>) {
        remove_view_cas_pipeline(&mut commands, view.0); // 与生产 removal 分支复用完全相同的清理入口。
    }

    #[test]
    fn live_render_view_still_removes_cas_pipeline() {
        let mut app = App::new(); // 使用真实 Commands 延迟应用语义验证存活实体路径。
        let view = app
            .world_mut()
            .spawn(ViewCasPipeline(CachedRenderPipelineId::INVALID))
            .id(); // 构造仍存活且持有 CAS 管线的 Render View。
        app.insert_resource(TestCasView(view)); // 将目标实体交给测试清理系统。
        app.add_systems(Update, remove_test_cas_pipeline); // 在一次正常 schedule 中应用延迟命令。

        app.update(); // 执行清理并刷新 Commands。

        assert!(app.world().get::<ViewCasPipeline>(view).is_none()); // `try_remove` 对活实体仍保持原有清理效果。
    }

    #[test]
    fn despawned_render_view_cleanup_is_silent() {
        let mut app = App::new(); // 使用真实 Commands 延迟应用语义复现销毁竞态。
        let view = app
            .world_mut()
            .spawn(ViewCasPipeline(CachedRenderPipelineId::INVALID))
            .id(); // 先构造曾经拥有 CAS 管线的 Render View。
        app.world_mut().despawn(view); // 模拟同步世界在 CAS Prepare 前已经销毁 Render View。
        app.insert_resource(TestCasView(view)); // removal reader 仍可能交付这一陈旧实体身份。
        app.add_systems(Update, remove_test_cas_pipeline); // 通过生产 helper 排队清理命令。

        app.update(); // 已销毁目标的清理必须安全完成；具体静默 API 由下一条源码契约锁定。
    }

    #[test]
    fn stale_render_view_cleanup_uses_silenced_entity_command() {
        let source = include_str!("mod.rs"); // 读取当前生产模块，避免只用 smoke 测试而漏掉 warn handler 差异。
        let helper_start = source
            .find("fn remove_view_cas_pipeline(")
            .expect("CAS 清理 helper 必须存在"); // 从生产 helper 起点切片，排除测试断言自身的字符串。
        let helper_end = source[helper_start..]
            .find("#[derive(Component)]")
            .map(|offset| helper_start + offset)
            .expect("CAS 清理 helper 必须位于管线组件之前"); // 不绑定 LF/CRLF，只审计真实生产 helper 区间。
        let helper = &source[helper_start..helper_end]; // 精确限定到 removal 分支实际调用的 helper。

        assert!(helper.contains(".try_remove::<ViewCasPipeline>()")); // Bevy 0.19 只有 try_remove 对已销毁实体使用 silenced command。
        assert!(!helper.contains(".remove::<ViewCasPipeline>()")); // 禁止回退到会固定调用 warn handler 的普通 remove。
    }
}
