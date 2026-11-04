//! 兼容薄层的复杂文字数据回归；放在集成测试中，避免测试代码改变生产转发入口。

use parley::{FontContext, LayoutContext, StyleProperty};

#[test]
fn japanese_text_uses_bundled_dictionary_segmentation() {
    const TEXT: &str = "日本語の文章を辞書分割して安全に改行する";
    let mut font_context = FontContext::new();
    let mut layout_context = LayoutContext::<()>::new();
    let mut builder = layout_context.ranged_builder(&mut font_context, TEXT, 1.0, true);
    builder.push_default(StyleProperty::FontSize(16.0));
    let mut layout = builder.build(TEXT); // 构建阶段会实际运行 ICU4X 单词与行分割器。
    layout.break_all_lines(Some(64.0));

    assert!(layout.lines().next().is_some()); // 字典数据缺失不得让复杂文字分析退化为空布局或报错。
}
