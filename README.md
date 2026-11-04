# Feathrise 第三方补丁

这里保存 SDK 使用的八个第三方 Rust 包修改版，以及 FidelityFX SDK 的原生补丁与来源材料。各目录保留上游包名、版本、原许可证和已有修改说明。整个目录作为独立 Git 仓交付。

外部工程在根清单的 `[patch.crates-io]` 中，用同一个补丁仓 URL 和完整提交配置 `avian3d`、`bevy_anti_alias`、`bevy_core_pipeline`、`bevy_render`、`dlss_wgpu`、`parley`、`wgpu-ffx`、`wgpu-hal`。补丁包通过 Git 提供，SDK 自有包通过 crates.io 提供。

SDK 和补丁仓同级检出后，在 SDK 目录使用 `cargo check --locked -p bevy_feathrise` 进行本地检查。

FidelityFX 材料位于 `fidelityfx-sdk-patches/`，四份原件参与已锁定原生包的精确身份校验，必须保留原始字节。SDK 的维护者源码重建和主仓组包通过 `-PatchRoot` 或 `FEATHRISE_FSR3_PATCH_ROOT` 显式指定该目录；普通 FSR 构建只读取用户提供的原生 ZIP。原件 README 中的旧命令作为来源记录保留，当前构建命令见 SDK 的 `crates/feathrise_fsr3_frame_generation/prebuilt/README.md`。
