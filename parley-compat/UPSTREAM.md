# Parley 兼容薄层

- 上游包：`parley 0.10.0`
- 上游仓库：<https://github.com/linebender/parley>
- 许可证：MIT OR Apache-2.0
- 本地包身份：`parley 0.9.0`，用于满足 Bevy 0.19 的依赖范围。

## 本地维护内容

本目录不复制 Parley 源码，只把旧包身份和 feature 转发到固定的 0.10.0 上游包，并始终启用官方 `complex-scripts` feature。该 feature 使用 ICU4X 字典数据处理 CJK、泰文、高棉文、老挝文和缅甸文，避免 Parley 0.9 的轻量分词器在中文 UI 每次布局时输出 `No segmentation model for language: ja`。

升级 Bevy 后，若其原生 Parley 版本已经提供并启用 `complex-scripts`，应删除本目录、工作区成员和根清单中的 `[patch.crates-io]` 条目。
