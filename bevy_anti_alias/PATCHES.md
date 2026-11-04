# Bevy Anti-Alias 0.19.1 本地补丁

本目录以 crates.io 的 `bevy_anti_alias 0.19.1` 为基线，许可证仍使用同目录中的 MIT / Apache-2.0 原文件。项目通过根 `Cargo.toml` 的 `[patch.crates-io]` 固定使用本镜像。

`rustfmt.toml` 同步 Bevy 0.19 上游的稳定格式选项，避免项目更新后的 rustfmt 对整份镜像产生与功能无关的 import 重排。

## 当前差异

- DLSS 准备阶段同时为相机主纹理追加 `STORAGE_BINDING` 与 `COPY_DST`。NGX 在求值前会清空输出，Vulkan 验证层因此要求主纹理除存储写入外还声明传输目标用途。
- `contrast_adaptive_sharpening::prepare_cas_pipelines` 在消费 `RemovedComponents<CasUniform>` 时，使用 `try_remove::<ViewCasPipeline>()` 代替普通 `remove`。
- 原因：玩法相机或 Render View 销毁时，同步世界可能先删除 render entity，随后 CAS Prepare 才消费旧实体的 removal。普通延迟命令会输出 `Entity despawned` 警告；`try_remove` 对仍存活实体照常清理，对已经销毁的实体静默结束。
- 边界：不修改 CAS 参数、shader、管线专用化、画质租约或输出像素，只修正销毁竞态下的清理行为。
- 回归测试锁定“存活实体仍移除管线”“已销毁实体清理安全完成”，并以生产 helper 源码契约精确禁止回退到会触发固定 warn handler 的普通 `remove`。
- DLSS RR 新增公开相机标记 `DlssRayReconstructionInputColorDebug`：RR 仍按正常 Quality / DLAA 路径执行，随后把双缓冲中保留的同帧 `pInColor` 以最近邻方式呈现到当前 HDR 输出，用于隔离“RR 前输入已有黑样本”与“RR 阶段产生黑样本”。
- RR 前颜色诊断不翻转 `ViewTarget`、不读回、不改变 guide、抖动、质量档、上下文或时间历史；Quality 仅按 `MainPassResolutionOverride` 有效子矩形放大，DLAA 为逐像素一一对应。该诊断会额外执行一次全屏呈现，不得用于正式性能数据。
- 为满足项目提交检查，三个上游 WGSL 文件中既有的 14 处行尾空格已移除；仅规范空白，不改变 shader token 或运行语义。
- `dlss::prepare_dlss` 不再对上下文创建错误调用 `expect`。相同输出尺寸、质量档和创建标志的失败会在 Render World 记录一次并移除旧上下文/分辨率覆盖；窗口尺寸或模式变化后允许重新查询，从而让不支持档位非致命回退且避免稳定帧日志刷屏。
- SR/RR 节点的 NGX 求值错误同样改为非致命遥测失败；只有 NGX 命令成功录制后才调用 `ViewTarget::post_process_write` 发布备用颜色。求值失败的当前帧继续使用原主颜色，下一主世界帧再移除组件并归还普通画质租约，避免回退边界出现黑帧或未写入目标。
- `DlssRuntimeTelemetry` 以主世界/RenderApp共享的小型快照分别发布SR与RR的输出尺寸、SDK最佳内部尺寸、明确模式和最近创建失败；设置与Solari桥只有在真实上下文成功回传后才宣称DLSS生效。该遥测不回读纹理、不增加GPU查询，也不允许调用方伪造模式可用性。

## 升级门

升级 Bevy 时必须重新对照上游 `prepare_cas_pipelines`：如果新版已经使用 fallible / silenced entity command，应删除对应 CAS 补丁；同时重新核对 DLSS 上下文错误传播、DLSS RR 的调度位置、`ViewTarget` 双缓冲语义、HDR 主纹理格式与 `MainPassResolutionOverride` 子矩形契约，再决定是否移植本地补丁。禁止直接沿用 0.19 快照。
