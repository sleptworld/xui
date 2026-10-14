// Glyph and rectangle quads. Glyphs are sampled from atlas pages: a coverage
// page and a colour page are bound together, so a run of text that mixes
// masks and colour glyphs is one draw.
//
// Each instance is one glyph or one solid rectangle: its quad in clip space,
// its colour, and the clip it is cut to in target pixels. Glyph quads are
// pixel aligned and as large as their bitmap, so texels are loaded, not
// filtered. A rectangle's quad covers every pixel it touches, and its edges
// are antialiased by the area of each pixel they cover.
//
// Colours are sRGB-encoded with straight alpha, as UI colours are. `LINEAR`
// says whether the target holds linear light (a float or sRGB format): they
// are decoded for it, and then blend in linear light.

override LINEAR: bool = false;

struct Instance {
    // Quad corners in clip space: x0, y0 (top left), x1, y1 (bottom right).
    @location(0) rect: vec4<f32>,
    // A glyph's texels in the page, or a rectangle's exact edges in target
    // pixels: x0, y0, x1, y1.
    @location(1) texels: vec4<f32>,
    // sRGB-encoded, straight alpha.
    @location(2) color: vec4<f32>,
    // Target pixels outside of x0, y0, x1, y1 are not drawn.
    @location(3) clip: vec4<f32>,
    // 0: coverage mask, 1: subpixel (RGB) mask, 2: colour bitmap, 3: rectangle.
    @location(4) kind: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texel: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) clip: vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
    @location(4) @interpolate(flat) shape: vec4<f32>,
}

// One coverage byte per texel: masks.
@group(0) @binding(0) var alpha_page: texture_2d<f32>;
// RGBA: subpixel masks and colour bitmaps.
@group(0) @binding(1) var color_page: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, instance: Instance) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex];
    var out: VertexOutput;
    out.position = vec4<f32>(mix(instance.rect.xy, instance.rect.zw, corner), 0.0, 1.0);
    out.texel = mix(instance.texels.xy, instance.texels.zw, corner);
    out.color = instance.color;
    out.clip = instance.clip;
    out.kind = instance.kind;
    out.shape = instance.texels;
    return out;
}

fn decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// `color` covering `coverage` of a pixel, premultiplied, as the target holds
// it: encoded values, or linear light.
fn ink(color: vec4<f32>, coverage: f32) -> vec4<f32> {
    let alpha = color.a * coverage;
    if LINEAR {
        return vec4<f32>(decode(color.rgb) * alpha, alpha);
    }
    return vec4<f32>(color.rgb * alpha, alpha);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = in.position.xy;
    if pixel.x < in.clip.x || pixel.y < in.clip.y || pixel.x >= in.clip.z || pixel.y >= in.clip.w {
        discard;
    }
    switch in.kind {
        case 0u: {
            return ink(in.color, textureLoad(alpha_page, vec2<i32>(floor(in.texel)), 0).r);
        }
        case 1u: {
            // Without dual-source blending a subpixel mask cannot weight each
            // channel of the destination, so its mean coverage is used.
            let texel = textureLoad(color_page, vec2<i32>(floor(in.texel)), 0);
            return ink(in.color, (texel.r + texel.g + texel.b) / 3.0);
        }
        case 2u: {
            // Colour bitmaps carry their own colour, premultiplied and
            // encoded; the run only fades them.
            let texel = textureLoad(color_page, vec2<i32>(floor(in.texel)), 0);
            if !LINEAR {
                return texel * in.color.a;
            }
            let straight = select(texel.rgb / texel.a, vec3<f32>(0.0), texel.a <= 0.0);
            return vec4<f32>(decode(straight) * texel.a, texel.a) * in.color.a;
        }
        default: {
            let edges = in.shape;
            let x = clamp(min(pixel.x + 0.5, edges.z) - max(pixel.x - 0.5, edges.x), 0.0, 1.0);
            let y = clamp(min(pixel.y + 0.5, edges.w) - max(pixel.y - 0.5, edges.y), 0.0, 1.0);
            return ink(in.color, x * y);
        }
    }
}
