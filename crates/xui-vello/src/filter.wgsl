// Layer filters, one full-screen pass per operation.
//
// A tap is an integer load. Each input says where its texel (0, 0) sits and
// how much of it holds the resource (`in0`..`in2`), and the output says where
// its own sits (`origin`), all in physical pixels of one coordinate space; so a
// texture can be larger than what it holds (a render-graph slot reused for a
// smaller resource), and anything outside what it holds reads as transparent.
// Left at their defaults, every texture is aligned with the output pixel for
// pixel and read whole.
//
// One colour space throughout: sRGB-encoded values, premultiplied, in
// half-float textures; the space Vello blends in inside a band. Vello's output
// (straight alpha) is premultiplied on the way in (op 0) and unpremultiplied on
// the way out (op 20), and nothing in between converts: joining bands (op 17),
// layer and backdrop composites, blend modes and every filter work on the
// values as they are, so cutting a band, isolating a layer or drawing text in
// one place rather than another never changes a colour. The one conversion is
// of what a texture sampler hands back as linear light (an sRGB or float
// external texture, op 18), which is encoded once, as it is drawn.

struct Params {
    // x: the operation; y: blend mode; z: mask kind (backdrop composite).
    op: vec4u,
    p0: vec4f,
    // x, y: the texture's origin in layer-local physical pixels; z: scale;
    // w: opacity (backdrop composite).
    p1: vec4f,
    p2: vec4f,
    p3: vec4f,
    color: vec4f,
    // Inverse of the mask transform: (xx, yx, xy, yy) and (dx, dy).
    inv0: vec4f,
    inv1: vec4f,
    m: array<vec4f, 5>,
    // Texture composite: the inverse placement (xx, yx, xy, yy); then (dx, dy,
    // opacity, clip count); then the texture's bounds x0 y0 x1 y1 in its own
    // space. `op.y` holds flags: bit 0 sRGB-encode, bit 1 nearest sampling.
    // Layer and backdrop composites use the clip count, and the clips, too.
    tex0: vec4f,
    tex1: vec4f,
    tex2: vec4f,
    // Four clips, four vec4 each: inverse (xx, yx, xy, yy); (dx, dy, kind,
    // radius); the rect x0 y0 x1 y1; padding.
    clips: array<vec4f, 16>,
    // Where the output's texel (0, 0) sits, in physical pixels.
    origin: vec4i,
    // Per input: where its texel (0, 0) sits, and the extent it holds.
    in0: vec4i,
    in1: vec4i,
    in2: vec4i,
    // The clip mask (`clip_mask`): where its texel (0, 0) sits in the
    // output's texels, and its extent; no mask when the extent is zero.
    mask_rect: vec4i,
}

@group(0) @binding(0) var src0: texture_2d<f32>;
@group(0) @binding(1) var src1: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> u: Params;
@group(0) @binding(4) var src2: texture_2d<f32>;
// Coverage of the clips the shader cannot evaluate itself (a path, or more
// than four), drawn by Vello, in its alpha.
@group(0) @binding(5) var clip_mask: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let positions = array<vec2f, 3>(vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0));
    return vec4f(positions[index], 0.0, 1.0);
}

// Texel `q` of an input, transparent outside what it holds.
fn fetch0(q: vec2i) -> vec4f {
    let size = min(vec2i(textureDimensions(src0)), u.in0.zw);
    if any(q < vec2i(0)) || any(q >= size) { return vec4f(0.0); }
    return textureLoad(src0, q, 0);
}

fn fetch1(q: vec2i) -> vec4f {
    let size = min(vec2i(textureDimensions(src1)), u.in1.zw);
    if any(q < vec2i(0)) || any(q >= size) { return vec4f(0.0); }
    return textureLoad(src1, q, 0);
}

// What an input holds at output texel `p`.
fn load0(p: vec2i) -> vec4f { return fetch0(p + u.origin.xy - u.in0.xy); }
fn load1(p: vec2i) -> vec4f { return fetch1(p + u.origin.xy - u.in1.xy); }

// Input 0 filtered bilinearly at `t`, in its texels (centres at +0.5), with
// transparent beyond its edge rather than the edge repeated.
fn bilinear0(t: vec2f) -> vec4f {
    let s = t - vec2f(0.5);
    let base = floor(s);
    let f = s - base;
    let i = vec2i(base);
    let top = mix(fetch0(i), fetch0(i + vec2i(1, 0)), f.x);
    let bottom = mix(fetch0(i + vec2i(0, 1)), fetch0(i + vec2i(1, 1)), f.x);
    return mix(top, bottom, f.y);
}

// One pass of a Gaussian. It reads at most 128 pixels either side
// (`MAX_PASS_RADIUS_PX` in `xui-render-graph`); the host cuts a larger blur into
// several passes with `gaussian_stages`, so this never truncates one.
fn gaussian(p: vec2i, axis: vec2i) -> vec4f {
    let sigma = max(u.p0.x, 0.0001);
    let radius = min(i32(ceil(sigma * u.p0.y)), 128);
    var sum = vec4f(0.0);
    var total = 0.0;
    for (var i = -radius; i <= radius; i += 1) {
        let w = exp(-0.5 * f32(i * i) / (sigma * sigma));
        sum += load0(p + axis * i) * w;
        total += w;
    }
    return sum / total;
}

fn over(front: vec4f, back: vec4f) -> vec4f {
    let af = clamp(front.a, 0.0, 1.0);
    let ab = clamp(back.a, 0.0, 1.0);
    return vec4f(front.rgb + back.rgb * (1.0 - af), af + ab * (1.0 - af));
}

fn unpremultiply(v: vec4f) -> vec4f {
    return select(vec4f(0.0), vec4f(v.rgb / v.a, v.a), v.a > 0.000001);
}

fn premultiply(v: vec4f) -> vec4f {
    return vec4f(v.rgb * v.a, v.a);
}


// --- Blend modes ----------------------------------------------------------

fn lum(c: vec3f) -> f32 { return dot(c, vec3f(0.3, 0.59, 0.11)); }
fn sat(c: vec3f) -> f32 { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
fn clip_color(c0: vec3f) -> vec3f {
    let l = lum(c0); let n = min(c0.r, min(c0.g, c0.b)); let x = max(c0.r, max(c0.g, c0.b));
    var c = c0;
    if n < 0.0 { c = vec3f(l) + (c - vec3f(l)) * l / (l - n); }
    if x > 1.0 { c = vec3f(l) + (c - vec3f(l)) * (1.0 - l) / (x - l); }
    return c;
}
fn set_lum(c: vec3f, l: f32) -> vec3f { return clip_color(c + vec3f(l - lum(c))); }
fn set_sat(c: vec3f, s: f32) -> vec3f {
    let lo = min(c.r, min(c.g, c.b)); let hi = max(c.r, max(c.g, c.b));
    if hi <= lo { return vec3f(0.0); }
    return (c - vec3f(lo)) * s / (hi - lo);
}
fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
    let d = select(sqrt(b), ((16.0 * b - 12.0) * b + 4.0) * b, b <= 0.25);
    return b + (2.0 * s - 1.0) * (d - b);
}
fn blend(mode: u32, b: vec3f, s: vec3f) -> vec3f {
    if mode == 0u { return s; }
    if mode == 1u { return b * s; }
    if mode == 2u { return b + s - b * s; }
    if mode == 3u { return select(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), b > vec3f(0.5)); }
    if mode == 4u { return min(b, s); }
    if mode == 5u { return max(b, s); }
    if mode == 6u { return select(min(vec3f(1.0), b / max(vec3f(0.00001), 1.0 - s)), vec3f(1.0), s >= vec3f(1.0)); }
    if mode == 7u { return select(1.0 - min(vec3f(1.0), (1.0 - b) / max(s, vec3f(0.00001))), vec3f(0.0), s <= vec3f(0.0)); }
    if mode == 8u { return select(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), s > vec3f(0.5)); }
    if mode == 9u { return vec3f(soft_light(b.r, s.r), soft_light(b.g, s.g), soft_light(b.b, s.b)); }
    if mode == 10u { return abs(b - s); }
    if mode == 11u { return b + s - 2.0 * b * s; }
    if mode == 12u { return set_lum(set_sat(s, sat(b)), lum(b)); }
    if mode == 13u { return set_lum(set_sat(b, sat(s)), lum(b)); }
    if mode == 14u { return set_lum(b, lum(s)); }
    return set_lum(s, lum(b));
}

// Source over destination with a blend mode, both premultiplied.
fn composite(source: vec4f, destination: vec4f, mode: u32) -> vec4f {
    let sa = clamp(source.a, 0.0, 1.0);
    let da = clamp(destination.a, 0.0, 1.0);
    let sc = select(vec3f(0.0), source.rgb / sa, sa > 0.000001);
    let dc = select(vec3f(0.0), destination.rgb / da, da > 0.000001);
    let blended = (1.0 - da) * sc + da * blend(mode, dc, sc);
    let alpha = sa + da * (1.0 - sa);
    let premul = sa * blended + da * (1.0 - sa) * dc;
    return vec4f(premul, alpha);
}

// Linear light as sRGB-encoded values: for what a sampler decoded.
fn srgb_encode(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - vec3f(0.055);
    return select(hi, lo, c <= vec3f(0.0031308));
}

// Coverage of a pixel by a shape, from the signed distance at its centre: a
// ramp one pixel wide, as a box filter gives, so an edge on a pixel boundary
// covers the pixel inside it fully. The width comes from derivatives, which
// keeps it one pixel under any scale.
fn shape_coverage(signed_distance: f32) -> f32 {
    let w = max(fwidth(signed_distance), 0.00001);
    return clamp(0.5 - signed_distance / w, 0.0, 1.0);
}

fn mask_point(logical: vec2f) -> vec2f {
    return vec2f(
        u.inv0.x * logical.x + u.inv0.z * logical.y + u.inv1.x,
        u.inv0.y * logical.x + u.inv0.w * logical.y + u.inv1.y
    );
}

fn mask_alpha(logical: vec2f) -> f32 {
    let kind = u.op.z;
    if kind == 0u { return 1.0; }
    let p = mask_point(logical);
    if kind == 1u {
        let q = abs(p - vec2f(0.5)) - vec2f(0.5);
        return shape_coverage(length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0));
    }
    if kind == 2u {
        let radius = clamp(u.p0.w, 0.0, 0.5);
        let q = abs(p - vec2f(0.5)) - vec2f(0.5 - radius);
        return shape_coverage(length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - radius);
    }
    if kind == 3u || kind == 4u {
        return shape_coverage(length((p - vec2f(0.5)) * 2.0) - 1.0);
    }
    if kind == 5u {
        let a = u.p0.xy; let b = u.color.xy; let ab = b - a;
        let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 0.00001), 0.0, 1.0);
        return shape_coverage(length(p - (a + ab * t)) - max(u.p0.z, 0.5));
    }
    return select(
        0.0,
        textureSampleLevel(src2, linear_sampler, p, 0.0).a,
        all(p >= vec2f(0.0)) && all(p <= vec2f(1.0))
    );
}

fn clip_coverage(pos: vec2f) -> f32 {
    var coverage = 1.0;
    let count = u32(u.tex1.w);
    for (var i = 0u; i < count; i += 1u) {
        let a = u.clips[i * 4u];
        let b = u.clips[i * 4u + 1u];
        let r = u.clips[i * 4u + 2u];
        let q = vec2f(
            a.x * pos.x + a.z * pos.y + b.x,
            a.y * pos.x + a.w * pos.y + b.y
        );
        let center = (r.xy + r.zw) * 0.5;
        let half_size = (r.zw - r.xy) * 0.5;
        let radius = min(b.w, min(half_size.x, half_size.y));
        let d = abs(q - center) - (half_size - vec2f(radius));
        let distance = length(max(d, vec2f(0.0))) + min(max(d.x, d.y), 0.0) - radius;
        coverage *= shape_coverage(distance);
    }
    if u.mask_rect.z > 0 {
        let q = vec2i(floor(pos)) - u.mask_rect.xy;
        let inside = all(q >= vec2i(0)) && all(q < u.mask_rect.zw);
        let texel = clamp(q, vec2i(0), u.mask_rect.zw - vec2i(1));
        coverage *= select(0.0, textureLoad(clip_mask, texel, 0).a, inside);
    }
    return coverage;
}

// An external texture drawn over what is composed so far. The texture is
// premultiplied; the output is too, encoded like everything else.
//
// Everything is computed for every pixel and only the last line chooses: a
// clip's coverage uses derivatives, and a derivative taken after some pixels
// of a 2x2 quad have returned early is undefined, which shows up as wrong
// pixels exactly along the texture's edge when that edge splits a quad.
fn texture_over(pos: vec2f, below: vec4f) -> vec4f {
    let local = vec2f(
        u.tex0.x * pos.x + u.tex0.z * pos.y + u.tex1.x,
        u.tex0.y * pos.x + u.tex0.w * pos.y + u.tex1.y
    );
    let size = u.tex2.zw - u.tex2.xy;
    let uv = (local - u.tex2.xy) / size;
    let inside = all(uv >= vec2f(0.0)) && all(uv <= vec2f(1.0));
    let clamped = clamp(uv, vec2f(0.0), vec2f(1.0));
    var c: vec4f;
    if (u.op.y & 2u) != 0u {
        let dims = vec2f(textureDimensions(src1));
        c = textureLoad(src1, vec2i(min(floor(clamped * dims), dims - vec2f(1.0))), 0);
    } else {
        c = textureSampleLevel(src1, linear_sampler, clamped, 0.0);
    }
    if (u.op.y & 1u) != 0u {
        // Linear light, premultiplied: encode the colour, keep it premultiplied.
        let straight = select(vec3f(0.0), c.rgb / c.a, c.a > 0.000001);
        c = vec4f(srgb_encode(straight) * c.a, c.a);
    }
    c = c * (clamp(u.tex1.z, 0.0, 1.0) * clip_coverage(pos));
    return select(below, c + below * (1.0 - c.a), inside);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = vec2i(position.xy);
    let kind = u.op.x;
    if kind == 0u { return premultiply(load0(p)); }
    if kind == 1u { return gaussian(p, vec2i(1, 0)); }
    if kind == 2u { return gaussian(p, vec2i(0, 1)); }
    if kind == 3u {
        let v = unpremultiply(load0(p));
        let t = clamp(vec4f(
            dot(u.m[0], v), dot(u.m[1], v), dot(u.m[2], v), dot(u.m[3], v)
        ) + u.m[4], vec4f(0.0), vec4f(1.0));
        return premultiply(t);
    }
    if kind == 7u {
        let a = load0(p).a;
        return vec4f(a, a, a, a);
    }
    if kind == 8u || kind == 9u {
        let radius = min(i32(ceil(u.p0.x)), 128);
        let axis = select(vec2i(1, 0), vec2i(0, 1), kind == 9u);
        var a = 0.0;
        for (var i = -radius; i <= radius; i += 1) {
            a = max(a, load0(p + axis * i).a);
        }
        return vec4f(a, a, a, a);
    }
    if kind == 10u {
        // The shadow is the blurred alpha shifted by the offset, drawn under
        // the original.
        let original = load0(p);
        let shifted = p - vec2i(round(u.p0.xy));
        let a = clamp(u.color.a * load1(shifted).a, 0.0, 1.0);
        return over(original, vec4f(u.color.rgb * a, a));
    }
    if kind == 17u {
        // The band (straight alpha, as Vello writes it) over what is composed
        // so far: the source-over Vello does inside a band.
        return over(premultiply(load0(p)), load1(p));
    }
    if kind == 18u {
        return texture_over(position.xy, load0(p));
    }
    if kind == 20u {
        // Premultiplied -> straight, as Vello writes.
        return unpremultiply(load0(p));
    }
    if kind == 21u {
        // Premultiplied, as it is: an 8-bit copy.
        return load0(p);
    }
    if kind == 12u {
        // Crop. The source is premultiplied already.
        return load0(p + vec2i(u.p0.xy));
    }
    if kind == 14u {
        // The grid is the coordinate space's, not this texture's, so it does
        // not move with how the output is cropped.
        let block = max(u.p0.xy, vec2f(1.0));
        let global = position.xy + vec2f(u.origin.xy);
        let snapped = (floor(global / block) + vec2f(0.5)) * block;
        return load0(vec2i(floor(snapped)) - u.origin.xy);
    }
    if kind == 15u {
        // u.p1.xy: the refraction centre in this texture; p0: strength and
        // chromatic spread in pixels.
        let delta = position.xy - u.p1.xy;
        let distance = max(length(delta), 0.0001);
        let direction = delta / distance;
        let displacement = direction * u.p0.x * exp(-(distance / u.p1.z) * 0.02);
        let chroma = direction * u.p0.y;
        let r = load0(vec2i(position.xy + displacement + chroma)).r;
        let g = load0(vec2i(position.xy + displacement)).g;
        let b = load0(vec2i(position.xy + displacement - chroma)).b;
        let a = load0(vec2i(position.xy + displacement)).a;
        return vec4f(r, g, b, a);
    }
    if kind == 16u {
        let offset = vec2i(u.p0.xy);
        let center = load0(p);
        return vec4f(load0(p + offset).r, center.g, load0(p - offset).b, center.a);
    }
    if kind == 24u {
        // Input 0 over input 1, both premultiplied.
        let front = load0(p);
        return front + load1(p) * (1.0 - front.a);
    }
    if kind == 22u {
        // A layer composite: the layer (input 0, layer-local) placed in the
        // parent by the composite transform, whose inverse is `inv0`/`inv1`
        // (parent logical -> layer logical), clipped by `clips`. `p1`: scale in
        // z, opacity in w. Nothing returns early: clips use derivatives.
        // With `op.w` set, input 1 is a snapshot of the destination and the
        // blend mode `op.y` is applied here; otherwise the pipeline's own
        // attachment blend composes what this returns.
        let parent = (position.xy + vec2f(u.origin.xy)) / u.p1.z;
        let local = vec2f(
            u.inv0.x * parent.x + u.inv0.z * parent.y + u.inv1.x,
            u.inv0.y * parent.x + u.inv0.w * parent.y + u.inv1.y
        );
        let src = bilinear0(local * u.p1.z - vec2f(u.in0.xy))
            * (clamp(u.p1.w, 0.0, 1.0) * clip_coverage(position.xy));
        let dst = load1(p);
        return select(src, composite(src, dst, u.op.y), u.op.w == 1u);
    }
    if kind == 23u {
        // A backdrop composite: the filtered backdrop (input 0) through the
        // mask (`op.z`, input 2 for a texture), in parent logical space, and
        // the clips. Blend
        // and snapshot as for a layer composite. Nothing returns early: the
        // mask's coverage uses derivatives.
        let logical = (position.xy + vec2f(u.origin.xy)) / u.p1.z;
        let src = load0(p)
            * (clamp(u.p1.w, 0.0, 1.0) * mask_alpha(logical) * clip_coverage(position.xy));
        let dst = load1(p);
        return select(src, composite(src, dst, u.op.y), u.op.w == 1u);
    }
    // 11: multiply by a mask image placed in layer-local logical space.
    let logical = (u.p1.xy + position.xy) / u.p1.z;
    let local = vec2f(
        u.inv0.x * logical.x + u.inv0.z * logical.y + u.inv1.x,
        u.inv0.y * logical.x + u.inv0.w * logical.y + u.inv1.y
    );
    let inside = all(local >= vec2f(0.0)) && all(local <= vec2f(1.0));
    let mask = select(0.0, textureSampleLevel(src1, linear_sampler, local, 0.0).a, inside);
    return load0(p) * mask;
}
