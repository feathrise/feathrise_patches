# 本地 `wgpu-ffx` 镜像说明

- 上游：<https://github.com/cwfitzgerald/wgpu-ffx>
- 固定提交：`457121d84a340a387d7c60353178829ac755d500`
- 包版本：`0.1.0`
- 着色器来源：上游生成包 `wgpu-ffx-shaders-spv 0.1.0`，其 vendor 脚本固定 AMD FidelityFX SDK `1.1.4`
- 许可：MIT，原文保存在同目录 `LICENSE.MIT`

## 项目补丁

1. 把继承上游 workspace 的 Cargo 字段改为独立清单，确保本目录可由项目的 `[patch.crates-io]` 精确引用。
2. 从 `FsrContext::dispatch` 删除每帧 `push_error_scope` 与 `pollster::block_on(pop)`。该同步等待违反项目《帧关键路径与异步计算规范》；WGPU 验证错误继续由 Bevy/WGPU 的未捕获错误处理链报告，不能在帧路径阻塞 CPU 等待 GPU 或设备回调。
3. `pollster` 仅由上游 GPU smoke tests 使用，因此移动到 `dev-dependencies`，正式运行依赖图不再包含同步执行器。
4. 修正低分辨率运动矢量变体的 Accumulate 资源绑定：AMD 着色器要求读取 Prepare Inputs 生成的 `DILATED_MOTION_VECTORS`，原端口却反向绑定了应用输入；两者同为 `Rg16Float`，因此原错误不会触发 WGPU 格式校验，却会破坏运动边界的历史重投影。
5. 复现 AMD 自动曝光资源重映射：启用 `AUTO_EXPOSURE` 时，全部 `INPUT_EXPOSURE` 读取改用 Luma Pyramid 写入的 `FRAME_INFO`，不再错误读取默认曝光纹理；同时增加低/全分辨率运动矢量与自动曝光消费者的纯资源表回归测试，并让连续两帧 GPU dispatch 烟雾覆盖项目正式使用的 HDR + 自动曝光组合。

6. GPU dispatch 烟雾增加同一上下文内锐化 `0 → 25 → 50 → 100 → 0 → 75%` 切换、无限远反向 Z 和非 16 倍数窗口尺寸。独立测试有界等待每次 GPU 提交实际完成，以捕获设备失效；正式帧路径不增加等待。本机通过只证明隔离超分路径，不能替代光追、帧生成与游戏内容组合复现。

除以上六项外，`src/` 与上游固定提交保持一致。升级时必须重新审计 `GenerateReactive`、设备变体选择、纹理格式、资源用途和全部帧路径等待。
