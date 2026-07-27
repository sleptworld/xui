struct UiUniforms {
    viewport_size: vec4f,
    scale_factor: vec4f,
}

@group(0) @binding(0)
var<uniform> ui: UiUniforms;

struct VertexInput {
    @location(0) position: vec2f,
    @location(1) color: vec4f,
}

struct VertexOutput {
    @invariant @builtin(position) position: vec4f,
    @location(0) color: vec4f,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let ndc = input.position * ui.scale_factor.xy / ui.viewport_size.xy
        * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
    var output: VertexOutput;
    output.position = vec4f(ndc, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    return input.color;
}
