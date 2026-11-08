# Avian 0.8.0 本地补丁

当前以 crates.io 的 `avian3d 0.8.0` 为基线。上游已修复重新绑定时更新树代理、Custom 形状缩放和重复登记物理岛。广相继续以实体身份拒绝自身碰撞。层级自动绑定在命令提交时复核目标刚体，旧刚体退役后按当前父层级选择有效目标。

## 0.7.0 验收历史

源码来自 crates.io 的 `avian3d 0.7.0`，保留原始许可证、依赖和功能开关。
原发布包校验值：`35a30cb730c72527ffaca7f8806881c7e4acc70a4a74c54eb1ffb56c27f64235`。

编辑器暂停期间移动子地图，关闭编辑器恢复物理时，新接触可能同时具有
`STARTED_TOUCHING` 和 `STARTED_GENERATING_CONSTRAINTS` 标记。首次接触分支
已经登记约束和物理岛，必须同时消费两个标记，否则下一步会再次登记同一接触，
触发 `contact.island.is_none()` 断言。

本地补丁仅在 `src/collision/narrow_phase/system_param.rs` 的首次接触分支
清除 `STARTED_GENERATING_CONSTRAINTS`，保留原碰撞、求解和断言行为。
上游对应修复：[Avian PR #1068](https://github.com/avianphysics/avian/pull/1068)。
正式流程回归位于 `src/sub_map_runtime_tests.rs`：添加源地图 `a`、等待来源角色激活、
通过编辑器位置 Y 输入抬高子地图，再关闭编辑器，持续运行真实物理。

2026-10-11验证：上述真实GPU窗口流程推进30秒Avian物理时钟通过；
`rpg_scene`、`rpg_character`、`rpg_grounding`库回归641项通过，6项沿用原忽略设置。
日志分别为 `target/sub-map-physics-runtime.log`、`target/sub-map-physics-libs.log`。
