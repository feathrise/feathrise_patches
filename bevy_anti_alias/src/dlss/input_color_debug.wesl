import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct InputColorDebugUniform {
    input_extent: vec2<u32>,
}

@group(0) @binding(0) var input_color: texture_2d<f32>;
@group(0) @binding(1) var<uniform> debug_uniform: InputColorDebugUniform;

// Present the exact input texels with nearest-neighbor scaling so filtering cannot invent or hide
// isolated black samples. In DLAA this mapping is one-to-one; Quality expands the valid subrect.
@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let output_extent = vec2<f32>(textureDimensions(input_color));
    let input_extent = max(debug_uniform.input_extent, vec2<u32>(1u));
    let normalized_position = in.position.xy / output_extent;
    let input_position = min(
        vec2<u32>(normalized_position * vec2<f32>(input_extent)),
        input_extent - vec2<u32>(1u),
    );
    return textureLoad(input_color, vec2<i32>(input_position), 0);
}
