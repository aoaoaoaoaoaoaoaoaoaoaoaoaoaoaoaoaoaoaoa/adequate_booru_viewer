struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> VsOut {
    var out: VsOut;
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.pos = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn kawase_down(in: VsOut) -> @location(0) vec4f {
    let half_texel = 0.5 / vec2f(textureDimensions(tex));
    var color = textureSample(tex, samp, in.uv) * 4.0;
    color += textureSample(tex, samp, in.uv - half_texel);
    color += textureSample(tex, samp, in.uv + half_texel);
    color += textureSample(tex, samp, in.uv + vec2f(half_texel.x, -half_texel.y));
    color += textureSample(tex, samp, in.uv - vec2f(half_texel.x, -half_texel.y));
    return color / 8.0;
}

@fragment
fn kawase_up(in: VsOut) -> @location(0) vec4f {
    let t = 1.0 / vec2f(textureDimensions(tex));
    var color = textureSample(tex, samp, in.uv + vec2f(-t.x * 2.0, 0.0));
    color += textureSample(tex, samp, in.uv + vec2f(-t.x, t.y)) * 2.0;
    color += textureSample(tex, samp, in.uv + vec2f(0.0, t.y * 2.0));
    color += textureSample(tex, samp, in.uv + vec2f(t.x, t.y)) * 2.0;
    color += textureSample(tex, samp, in.uv + vec2f(t.x * 2.0, 0.0));
    color += textureSample(tex, samp, in.uv + vec2f(t.x, -t.y)) * 2.0;
    color += textureSample(tex, samp, in.uv + vec2f(0.0, -t.y * 2.0));
    color += textureSample(tex, samp, in.uv + vec2f(-t.x, -t.y)) * 2.0;
    return color / 12.0;
}

struct Mask {
    a_min: vec2f,
    a_max: vec2f,
    b_min: vec2f,
    b_max: vec2f,
    water_min: vec2f,
    water_max: vec2f,
    radius_a: f32,
    radius_b: f32,
    strength: f32,
    dim: f32,
    blur: f32,
    tide: f32,
    scroll_tilt: f32,
    _pad1: f32,
    lift_rects: array<vec4f, 4>,
    lift_grips: vec4f,
    quivers: array<Quiver, 4>,
    splashes: array<Splash, 32>,
    viewer_min: vec2f,
    viewer_max: vec2f,
    touches: array<Touch, 12>,
    reach: f32,
    meniscus_px: f32,
    refract_px: f32,
    ior_spread: f32,
    quiver_bulge: f32,
    quiver_pulse: f32,
    tremor_k: f32,
    tremor_omega: f32,
    tremor_amp: f32,
    tremor_fade: f32,
    tremor_reach: f32,
    bulge_px: f32,
    lift_bright: f32,
    wave_v: f32,
    wave_sigma: f32,
    wave_damp: f32,
    wave_spread: f32,
    source_gain: f32,
    height_retention: f32,
    tilt_gain: f32,
    t_panel: f32,
    r_panel: f32,
    r_wall: f32,
    shore_feather: f32,
    raft_rect: vec4f,
    raft_corners: vec4f,
    floor_rect: vec4f,
}

struct Quiver { rect: vec4f, touch: vec4f }

struct Splash { rect: vec4f, vitals: vec4f }

struct Touch { wave: vec4f }

const LIFT_RADIUS: f32 = 3.0;
const PLATE_FEATHER: f32 = 6.0;
const PLATE_LIFT_GAIN: f32 = 2.0;
const PLATE_DRY_GAIN: f32 = 5.0;
const SHALLOW_BULGE_GAIN: f32 = 0.18;
const SHALLOW_BRIGHT_GAIN: f32 = 0.35;
const SHALLOW_MASS_GAIN: f32 = 1.35;
override FIELD_SCALE: f32 = 2.0;
const FIELD_HEIGHT_CEIL: f32 = 48.0;
const FIELD_FLOW_CEIL: f32 = 18.0;

@group(0) @binding(0) var sharp_tex: texture_2d<f32>;
@group(0) @binding(1) var blur_tex: texture_2d<f32>;
@group(0) @binding(2) var comp_samp: sampler;
@group(0) @binding(3) var<uniform> mask: Mask;
@group(0) @binding(4) var water_tex: texture_2d<f32>;

fn sd_cut(px: vec2f, rect_min: vec2f, rect_max: vec2f, radius: f32) -> f32 {
    let center = (rect_min + rect_max) * 0.5;
    let half_size = (rect_max - rect_min) * 0.5 - radius;
    let q = abs(px - center) - half_size;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn crossing(shore_px: f32, src_x: f32) -> f32 {
    let src_water = step(mask.water_min.x, src_x);
    let same = 1.0 - abs(shore_px - src_water);
    return mix(mask.t_panel, 1.0, same);
}

fn prism(flow: vec2f) -> mat3x2f {
    let g = flow * mask.refract_px;
    let spread = mask.ior_spread;
    return mat3x2f(g * max(0.0, 1.0 - spread), g, g * (1.0 + spread));
}

fn hash21(p: vec2f) -> f32 { return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453); }

fn mosaic(px: vec2f) -> vec3f {
    let q = px / 42.0;
    let c = floor(q);
    let f = fract(q);
    let edge = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y)) * 42.0;
    let grout = 1.0 - smoothstep(1.15, 2.35, edge);
    let n = hash21(c);
    let m = hash21(c + vec2f(19.7, 3.1));
    let pulse = (0.5 + 0.5 * sin(c.x * 0.67 + c.y * 1.21 + mask.tide * 0.13)) * 0.014;
    let tile = vec3f(0.115, 0.087, 0.052)
        + vec3f(0.070, 0.052, 0.030) * n
        + vec3f(pulse * 1.3)
        + vec3f(0.026, 0.015, 0.005) * (m - 0.5);
    return mix(tile, vec3f(0.045, 0.036, 0.026), grout * 0.68);
}

fn palette_gate(rgb: vec3f, swatch: vec3f, reach: f32) -> f32 {
    return 1.0 - smoothstep(0.0, reach, length(rgb - swatch));
}

fn pool_floor(rgb: vec3f, px: vec2f, flow: vec2f, shore_px: f32) -> vec3f {
    let page = palette_gate(rgb, vec3f(0.047, 0.043, 0.035), 0.035);
    let surface = palette_gate(rgb, vec3f(0.067, 0.059, 0.047), 0.026) * 0.45;
    let gate = shore_px * max(page, surface);
    return mix(rgb, mosaic(px + flow * 1.6), gate * 0.58);
}

fn finite(x: f32) -> bool {
    return x == x && abs(x) < 1e20;
}

fn island(sd: f32) -> f32 {
    return 1.0 - smoothstep(-PLATE_FEATHER, PLATE_FEATHER, sd);
}

fn field_obstacle(px: vec2f) -> f32 {
    var block = max(
        island(sd_cut(px, mask.a_min, mask.a_max, mask.radius_a)),
        island(sd_cut(px, mask.b_min, mask.b_max, mask.radius_b)),
    );
    for (var i = 0u; i < 4u; i = i + 1u) {
        let g = abs(mask.lift_grips[i]);
        if (g <= 0.0) {
            continue;
        }
        let r = mask.lift_rects[i];
        block = max(block, island(sd_cut(px, r.xy, r.zw, LIFT_RADIUS)) * g);
    }
    return clamp(block, 0.0, 1.0);
}

fn lift_warp(px: vec2f, rect: vec4f, grow: f32) -> vec2f {
    let emin = rect.xy - vec2f(grow);
    let emax = rect.zw + vec2f(grow);
    let away = px - (emin + emax) * 0.5;
    let half_t = (rect.zw - rect.xy) * 0.5;
    let half_b = half_t + vec2f(grow);
    let s = max(half_b.x, half_b.y) / max(max(half_t.x, half_t.y), 1.0);
    return away / s - away;
}

fn quiver_omega(q: Quiver) -> f32 {
    return select(mask.tremor_omega, q.touch.w, q.touch.w > 0.0);
}

fn touch_flow(px: vec2f, center: vec2f, age: f32, amp: f32) -> vec2f {
    let zero = vec2f(0.0);
    let ray = px - center;
    let d = length(ray);
    let travel = mask.wave_v * age;
    if (abs(d - travel) > 4.0 * mask.wave_sigma + 0.05 * travel) {
        return zero;
    }
    let a = amp * exp(-age / mask.wave_damp) / sqrt(1.0 + d / mask.wave_spread);
    let dir = ray / max(d, 1e-3);
    let s = (d - travel) / mask.wave_sigma;
    return dir * (a * s * exp(-s * s * 0.5));
}

fn sane_height(x: f32) -> f32 {
    return clamp(select(0.0, x, finite(x)), -FIELD_HEIGHT_CEIL, FIELD_HEIGHT_CEIL);
}

fn field_coord(px: vec2f) -> vec2f {
    let dims = vec2f(textureDimensions(water_tex));
    return clamp(px / FIELD_SCALE - vec2f(0.5), vec2f(0.0), dims - vec2f(1.0));
}

fn cell_height(p: vec2i) -> f32 {
    let dims = vec2i(textureDimensions(water_tex));
    return sane_height(textureLoad(water_tex, clamp(p, vec2i(0), dims - vec2i(1)), 0).x);
}

fn sample_height(px: vec2f) -> f32 {
    let q = field_coord(px);
    let p = vec2i(floor(q));
    let f = fract(q);
    let h00 = cell_height(p);
    let h10 = cell_height(p + vec2i(1, 0));
    let h01 = cell_height(p + vec2i(0, 1));
    let h11 = cell_height(p + vec2i(1, 1));
    return mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
}

fn sample_visible_height(px: vec2f, center_h: f32) -> f32 {
    return mix(sample_height(px), center_h, field_obstacle(px));
}

fn field_flow(px: vec2f) -> vec2f {
    let center_h = sample_height(px);
    let hx = sample_visible_height(px + vec2f(FIELD_SCALE, 0.0), center_h)
        - sample_visible_height(px - vec2f(FIELD_SCALE, 0.0), center_h);
    let hy = sample_visible_height(px + vec2f(0.0, FIELD_SCALE), center_h)
        - sample_visible_height(px - vec2f(0.0, FIELD_SCALE), center_h);
    var flow = -vec2f(hx, hy) * (4.5 / FIELD_SCALE);
    let mag = length(flow);
    flow = flow * min(1.0, FIELD_FLOW_CEIL / max(mag, 1e-4));
    return flow;
}

@fragment
fn composite(in: VsOut) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(sharp_tex));
    let px = in.uv * size;
    let shore_px = smoothstep(-mask.shore_feather, mask.shore_feather, px.x - mask.water_min.x);

    let dist = min(
        sd_cut(px, mask.a_min, mask.a_max, mask.radius_a),
        sd_cut(px, mask.b_min, mask.b_max, mask.radius_b),
    );
    let outside = smoothstep(-1.0, 1.0, dist);

    var flow_num = vec2f(0.0);
    var tint_num = 0.0;
    var plate_mass = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let raw_g = mask.lift_grips[i];
        let g = abs(raw_g);
        if (g <= 0.0) {
            continue;
        }
        let shallow = select(0.0, 1.0, raw_g < 0.0);
        let rect = mask.lift_rects[i];
        let grow = mask.bulge_px * g * mix(1.0, SHALLOW_BULGE_GAIN, shallow);
        let emin = rect.xy - vec2f(grow);
        let emax = rect.zw + vec2f(grow);
        let erad = LIFT_RADIUS + grow;
        let bd = sd_cut(px, emin, emax, erad);
        let w = island(bd) * g;
        flow_num = flow_num + lift_warp(px, rect, grow) * w;
        tint_num = tint_num + mask.lift_bright * g * w * mix(1.0, SHALLOW_BRIGHT_GAIN, shallow);
        plate_mass = plate_mass + w * mix(1.0, SHALLOW_MASS_GAIN, shallow);
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        let grow = g * mask.quiver_bulge * (1.0 + mask.quiver_pulse * sin(quiver_omega(q) * mask.tide));
        let emin = q.rect.xy - vec2f(grow);
        let emax = q.rect.zw + vec2f(grow);
        let bd = sd_cut(px, emin, emax, LIFT_RADIUS + grow);
        let w = island(bd) * g;
        flow_num = flow_num + lift_warp(px, q.rect, grow) * w;
        tint_num = tint_num + mask.lift_bright * 0.5 * g * w;
        plate_mass = plate_mass + w;
    }
    let plate_lift = 1.0 - exp(-PLATE_LIFT_GAIN * plate_mass);
    let flow = flow_num / max(plate_mass, 1e-4) * plate_lift;
    let tint = 1.0 + tint_num / max(plate_mass, 1e-4) * plate_lift;
    let dry = outside * exp(-PLATE_DRY_GAIN * plate_mass);

    var water_flow = field_flow(px) * crossing(shore_px, px.x);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        // Capillary pull toward the hovering fingertip: still one scalar
        // surface deformation, split chromatically only at the final prism.
        let to_ptr = q.touch.xy - px;
        let span = length(to_ptr);
        let p = exp(-(span * span) / (mask.reach * mask.reach));
        let inside = clamp(-sd_cut(px, q.rect.xy, q.rect.zw, 2.0), 0.0, 1.0);
        let bend = p * inside * g;
        let tdir = to_ptr / max(span, 1.0);
        water_flow = water_flow - tdir * (mask.meniscus_px * bend);
    }
    var viewer_flow = vec2f(0.0);
    let vx0 = mask.viewer_min.x;
    let vx1 = mask.viewer_max.x;
    let vy0 = mask.viewer_min.y;
    let vy1 = mask.viewer_max.y;
    for (var i = 0u; i < 12u; i = i + 1u) {
        let touch = mask.touches[i].wave;
        let amp = touch.w;
        if (amp <= 0.0) {
            continue;
        }
        let c = touch.xy;
        let age = touch.z;
        viewer_flow = viewer_flow
            + touch_flow(px, c, age, amp)
            + touch_flow(px, vec2f(2.0 * vx0 - c.x, c.y), age, amp * mask.r_wall)
            + touch_flow(px, vec2f(2.0 * vx1 - c.x, c.y), age, amp * mask.r_wall)
            + touch_flow(px, vec2f(c.x, 2.0 * vy0 - c.y), age, amp * mask.r_wall)
            + touch_flow(px, vec2f(c.x, 2.0 * vy1 - c.y), age, amp * mask.r_wall);
    }
    let viewer_wet = 1.0 - smoothstep(
        -1.0,
        1.0,
        sd_cut(px, mask.viewer_min, mask.viewer_max, 0.0),
    );

    let lift_flow = flow * outside;
    let wet = prism(water_flow) * dry + prism(viewer_flow) * viewer_wet;
    let uv_r = in.uv + (lift_flow + wet[0]) / size;
    let uv_g = in.uv + (lift_flow + wet[1]) / size;
    let uv_b = in.uv + (lift_flow + wet[2]) / size;
    let r = textureSample(sharp_tex, comp_samp, uv_r).r;
    let g = textureSample(sharp_tex, comp_samp, uv_g).g;
    let b = textureSample(sharp_tex, comp_samp, uv_b).b;
    let a = textureSample(sharp_tex, comp_samp, in.uv + lift_flow / size).a;
    let floor_size = mask.floor_rect.zw - mask.floor_rect.xy;
    let floor_zone = 1.0 - smoothstep(
        -1.0,
        1.0,
        sd_cut(px, mask.floor_rect.xy, mask.floor_rect.zw, 0.0),
    );
    let floor_gate = shore_px
        * outside
        * (1.0 - viewer_wet)
        * select(0.0, floor_zone, min(floor_size.x, floor_size.y) > 1.0);
    let floored = pool_floor(vec3f(r, g, b), px, water_flow, floor_gate);
    let sharp = vec4f(floored * mix(1.0, tint, outside), a);

    let blurred = textureSample(blur_tex, comp_samp, in.uv);
    let base = mix(sharp, blurred, mask.blur);
    let frosted = vec4f(base.rgb * mask.dim, base.a);
    let veiled = mix(sharp, frosted, mask.strength);
    return mix(sharp, veiled, outside);
}
