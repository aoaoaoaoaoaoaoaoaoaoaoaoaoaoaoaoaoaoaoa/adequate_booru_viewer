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

override SIM_SCALE: f32 = 2.0;
override DT: f32 = 1.0 / 240.0;
override IMPULSE_GAIN: f32 = 1.0;
const LIFT_RADIUS: f32 = 3.0;
const PLATE_FEATHER: f32 = 6.0;
const SOURCE_LIFE: f32 = 0.22;
const BASIN_TAPER: f32 = 28.0;
const BASIN_GAIN: f32 = 0.5;
const JITTER_GAIN: f32 = 2.0; // broadband thwack strength (noise is ±0.5)
const SOURCE_CEIL: f32 = 72.0;
const SOURCE_KICK_CEIL: f32 = 96.0;
const H_CEIL: f32 = 48.0;
const V_CEIL: f32 = 1440.0;
const TILT_FORCE_CEIL: f32 = 48.0;
const RAFT_STIFFNESS: f32 = 420.0;
const KO_EPSILON: f32 = 0.018;
const KO_SHOCK_GAIN: f32 = 10.0;
const RAIL_DAMPING: f32 = 0.52;

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var dst_tex: texture_storage_2d<rg32float, write>;
@group(0) @binding(2) var<uniform> mask: Mask;

fn cell_px(p: vec2i) -> vec2f {
    return (vec2f(p) + vec2f(0.5)) * SIM_SCALE;
}

fn sd_cut(px: vec2f, rect_min: vec2f, rect_max: vec2f, radius: f32) -> f32 {
    let center = (rect_min + rect_max) * 0.5;
    let half_size = (rect_max - rect_min) * 0.5 - radius;
    let q = abs(px - center) - half_size;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn island(sd: f32) -> f32 { return 1.0 - smoothstep(-PLATE_FEATHER, PLATE_FEATHER, sd); }

fn finite(x: f32) -> bool { return x == x && abs(x) < 1e20; }

fn sane(x: f32, ceil: f32) -> f32 { return clamp(select(0.0, x, finite(x)), -ceil, ceil); }

fn soft_limiter(x: f32, ceil: f32) -> f32 { return ceil * x / (abs(x) + ceil); }

fn rail_damp(h: f32, v: f32) -> f32 {
    let strain = max(abs(h) / H_CEIL, abs(v) / V_CEIL);
    return mix(1.0, RAIL_DAMPING, smoothstep(0.62, 0.92, strain));
}

fn quiver_omega(q: Quiver) -> f32 { return select(mask.tremor_omega, q.touch.w, q.touch.w > 0.0); }

fn obstacle(px: vec2f) -> f32 {
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

fn load_state(p: vec2i, dims: vec2i) -> vec2f {
    let raw = textureLoad(src_tex, clamp(p, vec2i(0), dims - vec2i(1)), 0).xy;
    return vec2f(sane(raw.x, H_CEIL), sane(raw.y, V_CEIL));
}

fn wall_height(p: vec2i, dims: vec2i, h: f32) -> f32 {
    let q = clamp(p, vec2i(0), dims - vec2i(1));
    let b = obstacle(cell_px(q));
    return mix(load_state(q, dims).x, h, b);
}

fn source_shell(px: vec2f, rect: vec4f, age: f32, amp: f32) -> f32 {
    if (amp == 0.0 || age > SOURCE_LIFE) {
        return 0.0;
    }
    let d = sd_cut(px, rect.xy, rect.zw, LIFT_RADIUS);
    let hull = smoothstep(-PLATE_FEATHER, 0.0, d);
    let shell = exp(-0.5 * pow(max(d, 0.0) / max(mask.wave_sigma, 1.0), 2.0));
    let birth = 1.0 - smoothstep(0.0, SOURCE_LIFE, age);
    return amp * hull * shell * birth;
}

fn source_basin(px: vec2f, rect: vec4f, age: f32, amp: f32) -> f32 {
    if (amp == 0.0 || age > SOURCE_LIFE) {
        return 0.0;
    }
    let d = sd_cut(px, rect.xy, rect.zw, LIFT_RADIUS); let taper = max(BASIN_TAPER, mask.wave_sigma * 1.6);
    let prism = 1.0 - smoothstep(-taper, 0.0, d);
    let birth = 1.0 - smoothstep(0.0, SOURCE_LIFE, age);
    return amp * prism * birth * BASIN_GAIN;
}

// A spun tape strip dragging the water in contact with it (`drag` is the signed
// screen-y the tape head travels). Friction piles the bulk off the leading edge
// — a bulge shoved outward there — with a weaker suction trailing behind, the
// bow-wave/wake of a moving surface. Skewed positive, so it reads as a push.
fn source_drag(px: vec2f, rect: vec4f, age: f32, amp: f32, drag: f32) -> f32 {
    if (amp == 0.0 || age > SOURCE_LIFE) {
        return 0.0;
    }
    let cx = (rect.x + rect.z) * 0.5;
    let half_w = max((rect.z - rect.x) * 0.5, 4.0);
    let reach = max(mask.wave_sigma * 1.2, 8.0);
    let across = exp(-0.5 * (px.x - cx) * (px.x - cx) / (half_w * half_w));
    // Leading bulge: a crest centred just past the edge the tape drives toward.
    let lead_y = select(rect.y, rect.w, drag > 0.0) + drag * reach;
    let bulge = exp(-0.5 * (px.y - lead_y) * (px.y - lead_y) / (reach * reach));
    // Trailing suction off the opposite edge, shallower.
    let trail_y = select(rect.w, rect.y, drag > 0.0) - drag * reach * 0.5;
    let suck = exp(-0.5 * (px.y - trail_y) * (px.y - trail_y) / (reach * reach * 1.6));
    let birth = 1.0 - smoothstep(0.0, SOURCE_LIFE, age);
    return amp * birth * across * (bulge - 0.5 * suck);
}

fn hash12(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453);
}

// The whole sheet thwacked from beneath: zero-mean broadband velocity noise, one
// value per cell (Nyquist content). The KO term shreds the high-k within a few
// steps — a quick shimmer — while the uniform friction carries the sparse low-k
// residue out. Flat noise is broadband, so the solver does the spectral shaping.
fn source_jitter(px: vec2f, rect: vec4f, age: f32, amp: f32) -> f32 {
    if (amp == 0.0 || age > SOURCE_LIFE) {
        return 0.0;
    }
    let inside = clamp(-sd_cut(px, rect.xy, rect.zw, 0.0) / PLATE_FEATHER, 0.0, 1.0);
    let birth = 1.0 - smoothstep(0.0, SOURCE_LIFE, age);
    // Re-seed the field each frame off the tide, so it boils rather than
    // freezing into one fixed pattern when thwacks pile up on a fast scroll.
    // (Velocity noise integrates into height, so the surface still reads smooth.)
    let seed = px + fract(mask.tide) * vec2f(53.0, 97.0);
    return amp * inside * birth * JITTER_GAIN * (hash12(seed) - 0.5);
}

fn source(px: vec2f) -> f32 {
    var drive = 0.0;
    for (var i = 0u; i < 32u; i = i + 1u) {
        let splash = mask.splashes[i];
        if (abs(splash.vitals.w) > 0.5) { drive = drive + source_drag(px, splash.rect, splash.vitals.x, splash.vitals.y, splash.vitals.w); }
        else if (splash.vitals.z > 1.5) { drive = drive + source_jitter(px, splash.rect, splash.vitals.x, splash.vitals.y); }
        else if (splash.vitals.z > 0.5) { drive = drive + source_basin(px, splash.rect, splash.vitals.x, splash.vitals.y); }
        else { drive = drive + source_shell(px, splash.rect, splash.vitals.x, splash.vitals.y); }
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        let d = sd_cut(px, q.rect.xy, q.rect.zw, LIFT_RADIUS);
        if (d <= 0.0 || d > mask.tremor_reach) {
            continue;
        }
        let shell = exp(-d / max(mask.tremor_fade, 1.0));
        let phase = mask.tremor_k * d - quiver_omega(q) * mask.tide;
        drive = drive + mask.tremor_amp * g * shell * sin(phase);
    }
    return soft_limiter(drive, SOURCE_CEIL);
}

fn tilt_drive(px: vec2f) -> f32 {
    let span = max(mask.water_max.y - mask.water_min.y, 1.0);
    let y = clamp((px.y - mask.water_min.y) / span, 0.0, 1.0);
    let ramp = y * 2.0 - 1.0;
    let lip = max(SIM_SCALE * 2.0, 1.0);
    let y_gate = smoothstep(mask.water_min.y, mask.water_min.y + lip, px.y)
        * (1.0 - smoothstep(mask.water_max.y - lip, mask.water_max.y, px.y));
    let x_gate = smoothstep(
        mask.water_min.x - mask.shore_feather,
        mask.water_min.x + mask.shore_feather,
        px.x,
    );
    let force = -clamp(mask.scroll_tilt, -TILT_FORCE_CEIL, TILT_FORCE_CEIL) * ramp;
    return force * max(mask.tilt_gain, 0.0) * x_gate * y_gate;
}

fn raft_height(px: vec2f) -> f32 {
    let r = mask.raft_rect;
    let size = r.zw - r.xy;
    if (size.x <= 1.0 || size.y <= 1.0) {
        return 0.0;
    }
    let uv = clamp((px - r.xy) / size, vec2f(0.0), vec2f(1.0));
    let top = mix(mask.raft_corners.x, mask.raft_corners.y, uv.x);
    let bottom = mix(mask.raft_corners.w, mask.raft_corners.z, uv.x);
    let sheet = mix(top, bottom, uv.y);
    let gate = island(sd_cut(px, r.xy, r.zw, LIFT_RADIUS));
    return sheet * gate;
}

fn raft_drive(px: vec2f, h: f32) -> f32 {
    return (raft_height(px) - h) * RAFT_STIFFNESS;
}

@compute @workgroup_size(8, 8, 1)
fn step(@builtin(global_invocation_id) gid: vec3u) {
    let dims_u = textureDimensions(src_tex);
    if (gid.x >= dims_u.x || gid.y >= dims_u.y) {
        return;
    }
    let dims = vec2i(dims_u);
    let p = vec2i(gid.xy);
    let px = cell_px(p);
    let here = load_state(p, dims);
    let h = here.x;

    let l = wall_height(p + vec2i(-1, 0), dims, h);
    let r = wall_height(p + vec2i(1, 0), dims, h);
    let u = wall_height(p + vec2i(0, -1), dims, h);
    let d = wall_height(p + vec2i(0, 1), dims, h);
    let lap = (l + r + u + d - 4.0 * h) / (SIM_SCALE * SIM_SCALE);
    let lu = wall_height(p + vec2i(-1, -1), dims, h);
    let ru = wall_height(p + vec2i(1, -1), dims, h);
    let ld = wall_height(p + vec2i(-1, 1), dims, h);
    let rd = wall_height(p + vec2i(1, 1), dims, h);
    let ll = wall_height(p + vec2i(-2, 0), dims, h);
    let rr = wall_height(p + vec2i(2, 0), dims, h);
    let uu = wall_height(p + vec2i(0, -2), dims, h);
    let dd = wall_height(p + vec2i(0, 2), dims, h);
    let rough = clamp(abs(l + r + u + d - 4.0 * h) / (4.0 * H_CEIL), 0.0, 1.0);
    let ko_gain = KO_EPSILON * mix(1.0, KO_SHOCK_GAIN, smoothstep(0.18, 0.72, rough));
    let ko = (
        20.0 * h
            - 8.0 * (l + r + u + d)
            + 2.0 * (lu + ru + ld + rd)
            + ll + rr + uu + dd
    ) * (ko_gain / 64.0);

    let shelf = smoothstep(-mask.shore_feather, mask.shore_feather, px.x - mask.water_min.x);
    let shelf_speed = clamp((1.0 - mask.r_panel) / (1.0 + mask.r_panel), 0.2, 1.0);
    let cfl = 0.66 * SIM_SCALE / DT;
    let c = min(mask.wave_v * mix(shelf_speed, 1.0, shelf), cfl);
    let kick = soft_limiter(source(px) * mask.source_gain * IMPULSE_GAIN, SOURCE_KICK_CEIL);
    var v = here.y
        + c * c * lap * DT
        + kick
        + (tilt_drive(px) + raft_drive(px, h)) * DT;
    v = v * exp(-DT / max(mask.wave_damp, 0.08)) * mix(0.985, 1.0, shelf);
    v = v * rail_damp(h, v);

    let block = obstacle(px);
    v = mix(v, 0.0, block);
    let keep = clamp(mask.height_retention, 0.95, 1.0);
    var next_h = mix((h + v * DT) * keep - ko, h * keep, block);
    v = sane(v, V_CEIL);
    next_h = sane(next_h, H_CEIL);
    textureStore(dst_tex, p, vec4f(next_h, v, 0.0, 0.0));
}
