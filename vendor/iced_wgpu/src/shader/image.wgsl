struct Globals {
    transform: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var u_sampler: sampler;
@group(1) @binding(0) var u_texture: texture_2d_array<f32>;

// Anomi (`PATCHES.md`, "Marked pictures"): an instance's `flags`, as
// `image/mark.rs` sets them for a picture the app marked as premultiplied
// BGRA, tiny-skia's own format. Its bytes are uploaded as they are; the
// shader swaps red and blue back and doesn't premultiply them again.
const IMAGE_BGRA: u32 = 1u;
const IMAGE_PREMULTIPLIED: u32 = 2u;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) center: vec2<f32>,
    @location(1) clip_bounds: vec4<f32>,
    @location(2) border_radius: vec4<f32>,
    @location(3) tile: vec4<f32>,
    @location(4) rotation: f32,
    @location(5) opacity: f32,
    @location(6) atlas_pos: vec2<f32>,
    @location(7) atlas_scale: vec2<f32>,
    @location(8) layer: i32,
    @location(9) flags: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) clip_bounds: vec4<f32>,
    @location(1) @interpolate(flat) border_radius: vec4<f32>,
    @location(2) @interpolate(flat) atlas: vec4<f32>,
    @location(3) @interpolate(flat) layer: i32,
    @location(4) @interpolate(flat) opacity: f32,
    @location(5) uv: vec2<f32>,
    @location(6) @interpolate(flat) flags: u32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Generate a vertex position in the range [0, 1] from the vertex index
    let corner = vertex_position(input.vertex_index);

    let tile = input.tile;
    let center = input.center;

    // List the unrotated tile corners
    let corners = array<vec2<f32>, 4>(
        tile.xy,                              // Top left
        tile.xy + vec2<f32>(tile.z, 0.0),     // Top right
        tile.xy + vec2<f32>(0.0, tile.w),     // Bottom left
        tile.xy + tile.zw                     // Bottom right
    );

    // Rotate tile corners around center
    let cos_r = cos(-input.rotation); // Clockwise
    let sin_r = sin(-input.rotation);
    var rotated = array<vec2<f32>, 4>();

    for (var i = 0u; i < 4u; i++) {
        let c = corners[i] - input.center;
        rotated[i] = vec2<f32>(c.x * cos_r - c.y * sin_r, c.x * sin_r + c.y * cos_r) + input.center;
    }

    // Find bounding box of rotated tile
    var min_xy = rotated[0];
    var max_xy = rotated[0];
    for (var i = 1u; i < 4u; i++) {
        min_xy = min(min_xy, rotated[i]);
        max_xy = max(max_xy, rotated[i]);
    }
    let rotated_bounds = vec4<f32>(min_xy, max_xy - min_xy);

    // Intersect with clip bounds
    let clip_min = max(rotated_bounds.xy, input.clip_bounds.xy);
    let clip_max = min(rotated_bounds.xy + rotated_bounds.zw, input.clip_bounds.xy + input.clip_bounds.zw);
    let clipped_tile = vec4<f32>(clip_min, max(vec2<f32>(0.0), clip_max - clip_min));

    // Calculate the vertex position
    let v_pos = clipped_tile.xy + corner * clipped_tile.zw;
    out.position = vec4<f32>(v_pos, 0.0, 1.0);
    out.clip_bounds = input.clip_bounds;

    // Calculate rotated UV
    let uv = input.atlas_pos + (v_pos - tile.xy) / tile.zw * input.atlas_scale;
    let uv_center = input.atlas_pos + input.atlas_scale / 2.0;

    let d = uv - uv_center;
    out.uv = vec2<f32>(d.x * cos_r - d.y * sin_r, d.x * sin_r + d.y * cos_r) + uv_center;

    out.position = globals.transform * out.position;
    out.border_radius = min(input.border_radius, vec4(min(input.clip_bounds.z, input.clip_bounds.w) / 2.0));
    out.atlas = vec4(input.atlas_pos, input.atlas_pos + input.atlas_scale);
    out.layer = input.layer;
    out.opacity = input.opacity;
    out.flags = input.flags;

    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let fragment = input.position.xy;
    let position = input.clip_bounds.xy;
    let scale = input.clip_bounds.zw;

    // Anomi: the offset from the centre negated, as the quad shader has it,
    // for `rounded_box_sdf`'s corner order (top left, top right, bottom
    // right, bottom left). Stock, each corner took its opposite's radius: a
    // banner round at the top only came out round at the bottom. (Upstream
    // Yapfast's fix.)
    let d = rounded_box_sdf(
        -2.0 * (fragment - position - scale / 2.0),
        scale,
        input.border_radius * 2.0,
    ) / 2.0;

    // Anomi: the edge where the box is, as the quad shader and tiny-skia put
    // it: a pixel's coverage is half a pixel past its centre's distance.
    // Stock, `1.0 - d` put the edge half a pixel outside, which the clip
    // then cut: round avatars came out fuller, with flat sides. (Upstream
    // Yapfast's fix.)
    let antialias: f32 = clamp(0.5 - d, 0.0, 1.0);
    let inside = all(input.uv >= input.atlas.xy) && all(input.uv <= input.atlas.zw);
    let coverage = antialias * input.opacity * f32(inside);

    var sampled = textureSample(u_texture, u_sampler, input.uv, input.layer);

    // Anomi: a marked picture's texel is BGRA, premultiplied. (With
    // `web-colors` off the atlas is sRGB, and its premultiplied values are
    // linearized as if straight: a see-through edge comes out a little off.
    // Anomi builds with `web-colors`.)
    if (input.flags & IMAGE_BGRA) != 0u {
        sampled = sampled.bgra;
    }

    if (input.flags & IMAGE_PREMULTIPLIED) != 0u {
        return sampled * coverage;
    }

    return premultiply(sampled * vec4<f32>(1.0, 1.0, 1.0, coverage));
}

fn rounded_box_sdf(p: vec2<f32>, size: vec2<f32>, corners: vec4<f32>) -> f32 {
    let box_half = select(corners.yz, corners.xw, p.x > 0.0);
    let corner = select(box_half.y, box_half.x, p.y > 0.0);
    let q = abs(p) - size + corner;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - corner;
}
