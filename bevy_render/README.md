# Bevy Render

## 项目补丁（0.20.1）

- `view/window/screenshot::render_screenshot` 在截图回显管线尚未就绪时也向真实窗口附件编码空渲染pass。截图已写离屏附件时仍会触发Present，旧实现跳过窗口提交，导致Vulkan获取信号量没有被消费就再次复用（`VUID-vkAcquireNextImageKHR-semaphore-01286`）。修复复用原encoder，不等待异步编译、不增加提交或回读；显式顶栏验收在开启Vulkan校验下覆盖首次截图、最大化、最小化恢复与缩放。
- 驻留实体同步随上游搬到同仓的 `bevy_extract`。新版空网格分配与上传都跳过零顶点网格，可见性查询也已按主世界实体二分。
- `texture::GpuImage::prepare_asset` 复用纹理时按 `TextureDataOrder` 上传全部 Mip 与数组层，兼容压缩块及三维深度切片。旧逻辑仅上传 mip 0，地表保存后远处仍采样旧空白权重，形成随视角移动的白色分界；冷加载正常。保留纹理与视图身份，已有材质绑定即可看到完整更新。上传沿用 `RenderAsset::byte_len` 的完整资产预算，不新增像素构建、GPU 等待或回读。
- CPU 回归：根程序 `terrain_shading_runtime_tests::pending_material_survives_different_render_entity_order`。显式 GPU 回归 `gpu_new_map_surface_first_save` 覆盖真实新建、Clipmap、刷涂、保存、远近视角和关闭 F12；同次运行还逐级回读 Rgba8、Rgba32Uint、三维 R8，以及设备支持时的 BC7，覆盖两种数据顺序并检查纹理/视图复用。
- GPU 用例必须单独、单线程运行：`BEVY_ASSET_ROOT` 指向项目 `target` 内的隔离夹具根。夹具含复制的 `settings.json`、真实目录 `assets/config`（复制项目配置）与 `assets/maps`（初始化空目录表），其余只读资产可复制或目录链接；禁止将作者目录链接到用户资产。运行 `cargo test --locked --features test_panel --bin bevy_learn5 terrain_shading_runtime_tests -- --include-ignored --nocapture --test-threads=1`，截图输出到夹具 `target/terrain-first-save-*.png`。

[![License](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/bevyengine/bevy#license)
[![Crates.io](https://img.shields.io/crates/v/bevy_render.svg)](https://crates.io/crates/bevy_render)
[![Downloads](https://img.shields.io/crates/d/bevy_render.svg)](https://crates.io/crates/bevy_render)
[![Docs](https://docs.rs/bevy_render/badge.svg)](https://docs.rs/bevy_render/latest/bevy_render/)
[![Discord](https://img.shields.io/discord/691052431525675048.svg?label=&logo=discord&logoColor=ffffff&color=7389D8&labelColor=6A7EC2)](https://discord.gg/bevy)
