//! 补丁包颜色诊断 Shader 的 Naga 契约，不触发厂商 SDK 或 GPU 运行。

#[test]
fn input_color_debug_shader_is_valid_wgsl() {
    let shader =
        include_str!("../src/dlss/input_color_debug.wesl"); // 直接读取补丁包嵌入的生产 Shader，避免测试复制另一份实现。
    let validation_source = shader.replacen(
        "import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;",
        "struct FullscreenVertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, };",
        1,
    ); // naga不处理Bevy预处理指令，测试只以内联等价接口替换这一条正式import。
    let module = naga::front::wgsl::parse_str(&validation_source)
        .expect("RR前颜色诊断shader必须能通过WGSL解析"); // 覆盖整数坐标换算、纹理读取和uniform声明。
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(), // 检查完整表达式、绑定和片元阶段规则。
        naga::valid::Capabilities::all(),    // 使用完整桌面能力避免误拒绝标准WGSL语法。
    )
    .validate(&module)
    .expect("RR前颜色诊断shader必须通过naga语义验证"); // 运行时管线创建前先固定源码合法性。
}
