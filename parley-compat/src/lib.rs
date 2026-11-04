//! Bevy 0.19 使用的 Parley 0.9 包身份兼容层。
//!
//! 【功能设计】公共 API 完整转发到 Parley 0.10，并强制启用其官方
//! `complex-scripts` 字典分词；这样 Bevy 无需源码 fork 即可正确处理中文换行，
//! 同时消除 debug 构建中反复出现的 ICU4X `ja` 模型缺失提示。

#![cfg_attr(not(feature = "std"), no_std)]

pub use parley_upstream::*;
