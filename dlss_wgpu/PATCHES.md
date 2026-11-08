# 本地补丁记录

本目录以 crates.io 的 `dlss_wgpu 6.0.0` 为基线，对应 NVIDIA DLSS SDK `310.9.1` 与 `wgpu 30`。

## Super Resolution 固定路径使用最佳内部尺寸

上游创建 NGX 上下文时使用 `NGX_DLSS_GET_OPTIMAL_SETTINGS` 返回的最佳内部尺寸，却让固定分辨率调用方从 `render_resolution()` 读取动态范围下限，导致实际主通道尺寸与上下文创建契约不一致。

本地补丁保存并返回 `optimal_render_resolution`；动态范围 API 仍原样返回 `min..=max`。同时公开 `DlssSdk::query_super_resolution_settings`，调用方可以在创建上下文前查询模式/输出组合是否可用，并把 SDK 错误作为非致命回退原因处理。

## Vulkan NGX 求值跨命令缓冲同步

SDK 310.5.3 会在 SR/RR 求值内部以 `vkCmdClearColorImage` 清空输出，但没有为该传输写入与调用方前后的独立命令缓冲建立完整依赖。仅声明 `TRANSFER_DST` 能修复图像用途错误，仍会留下 `WRITE_AFTER_WRITE`，并可能继续破坏交换链同步。

本地补丁在每次 NGX 求值的同一原生命令缓冲首尾各记录一次不改变图像布局的全局 Vulkan 内存屏障：前置屏障连接 wgpu 的资源布局转换与 NGX 清空/计算，后置屏障连接 NGX 与后续 wgpu pass。图像布局和资源跟踪仍由 wgpu `transition_resources` 唯一管理；调用方还必须给 DLSS 输出声明 `COPY_DST`。

上游README仍明确声明DLSS自身会产生可安全忽略的Vulkan validation errors。本地RTX 4090实测中Performance与Ultra Performance首次求值仍会报告`VUID-vkCmdDraw-None-09600`的SDK内部图像`UNDEFINED -> GENERAL`问题；本项目不得全局屏蔽该VUID，因为相同验证规则也能发现项目自己的真实子资源布局缺陷。两档功能可运行不等于“零错误”验收通过，SDK或本crate升级后必须重测。

## Windows运行时只暴露SR与RR白名单DLL

上游运行时会把`DLSS_SDK/lib/Windows_x86_64/{rel|dev}`整个目录写入NGX的`PathListInfo`。SDK目录同时含有项目未采用的`nvngx_dlssg.dll`，NGX初始化时仍可能把该Frame Generation模块载入进程；外部Vulkan监控层会据此把本游戏误判为帧生成游戏并主动禁用叠加显示。

本地补丁由`build.rs`在Cargo私有`OUT_DIR/dlss_runtime`中只复制`nvngx_dlss.dll`与`nvngx_dlssd.dll`，并清除该生成目录内其他`nvngx_*`残留。运行时优先采用可执行文件同目录的精确两项白名单，普通`cargo run`回退构建期隔离目录；两者都拒绝包含Frame Generation或其他额外NGX运行库的目录，不再读取运行时`DLSS_SDK`环境变量。正式发行仍由`xtask`复制并审计相同两项DLL。

## Ray Reconstruction 创建标志

Bevy 0.20.1 在 RR 构造前去除 `AutoExposure`，dlss_wgpu 6.0.0 会拒绝带该标志的 RR 构造请求。升级后由这两处上游逻辑共同遵循 RR 的创建契约，Super Resolution 仍可使用自动曝光。

## NGX 绑定构建

绑定生成保留 Clang 动态加载与构建日志。生成的 Rust 绑定由编译器直接读取，关闭可选格式化，避免 `bindgen` 和 `prettyplease` 从不同的 `syn` 主版本传递语法树。

## 已撤销：Ray Reconstruction 真实帧间隔 A/B

曾把可选研究输入 `InFrameTimeDeltaInMsec` 从上游允许的 `0.0` 改为相邻成功求值的真实毫秒间隔。用户在 RTX/Vulkan 实机确认该实验会让杂粮明显发黑，同时等待后白光仍继续扩散，因此它既引入画质回归也没有解决原缺陷。

当前源码采用上游的固定 `0.0` 输入，延续此前撤销计时实验后的行为。
