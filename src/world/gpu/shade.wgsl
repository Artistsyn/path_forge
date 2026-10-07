// Deferred shading of the G-buffer: render::surface_uv, shade_px, light_among, gloss_of,
// ripple_slope and weather::surface/rain_rings, for one world. Two entry points: `uvs` works out
// every pixel's texture coordinates (shading reads its neighbours' for the footprint), `shade`
// lights each pixel and writes its colour and what the reflection pass needs.

@group(0) @binding(1) var<storage, read> gbuf: array<GPix>;
// Texture slots: a header of (offset, side) per slot and level, then linear RGB texels.
@group(0) @binding(2) var<storage, read> tex_words: array<u32>;
// Point lights (8 floats: pos, radius, colour, 0), then sky lights (dir, 0, colour, 0).
@group(0) @binding(3) var<storage, read> lights: array<f32>;
// Per-tile light lists: (n tiles + 1) offsets, then light indices.
@group(0) @binding(4) var<storage, read> tiles: array<u32>;
// Sun mask, then ambient-occlusion mask, one float per pixel each.
@group(0) @binding(5) var<storage, read> masks: array<f32>;
// u, v, texture (-1: nothing drawn) per pixel.
@group(0) @binding(6) var<storage, read_write> uvs: array<f32>;
@group(0) @binding(7) var<storage, read_write> hdr: array<f32>;
// Refl: r, env (3), rough, sx.
@group(0) @binding(8) var<storage, read_write> refl: array<f32>;

// The air's settings (mist); see render::gpu_air.
struct Air {
    mist_on: u32, density: f32, top: f32, patchy: f32,
    travel: f32, seed: u32, mist_r: f32, mist_g: f32,
    mist_b: f32, pad0: u32, pad1: u32, pad2: u32,
}
@group(0) @binding(9) var<uniform> A: Air;

const TEX_HEADER: u32 = 136u; // 8 slots x 8 levels x (offset, side), then 8 level counts.

fn tex_sample(slot: u32, u: f32, v: f32, footprint: f32) -> vec3<f32> {
    let levels = tex_words[128u + slot];
    let base = f32(tex_words[(slot * 8u) * 2u + 1u]);
    let texels = max(footprint * base, 1e-6);
    let lod = min(u32(max(log2(texels), 0.0)), levels - 1u);
    let e = (slot * 8u + lod) * 2u;
    let off = tex_words[e];
    let s = tex_words[e + 1u];
    let x = u32(rem_e(u, 1.0) * f32(s)) % s;
    let y = u32(rem_e(v, 1.0) * f32(s)) % s;
    let k = TEX_HEADER + off + (y * s + x) * 3u;
    return vec3<f32>(bitcast<f32>(tex_words[k]), bitcast<f32>(tex_words[k + 1u]), bitcast<f32>(tex_words[k + 2u]));
}

fn rot(bit: u32, u: f32, v: f32) -> vec2<f32> {
    if (W.rotate & bit) != 0u { return vec2<f32>(v, -u); }
    return vec2<f32>(u, v);
}

// render::surface_uv: (u, v, tex), tex < 0 for nothing.
fn surface_uv(g: GPix) -> vec3<f32> {
    let id = g.idr & 0xFFu;
    let dw = g.d + W.scroll;
    if id == ID_GROUND && on_bridge(g.d) {
        return vec3<f32>(rot(16u, div(g.x, W.deck_tile), div(dw, W.deck_tile)), 4.0);
    }
    if id == ID_CHASM { return vec3<f32>(div(g.x, W.bottom_tile), div(dw, W.bottom_tile), 5.0); }
    if id == ID_CLIFF {
        if W.walls_on != 0u { return vec3<f32>(div(g.x + dw, W.wall_tile), div(-g.y, W.wall_tile), 2.0); }
        if W.verge_on != 0u { return vec3<f32>(div(g.x, W.verge_tile), div(-g.y, W.verge_tile), 1.0); }
        return vec3<f32>(div(g.x, W.path_tile), div(-g.y, W.path_tile), 0.0);
    }
    if is_rail(id) {
        let m = (id - ID_RAIL) / 4u;
        let face = (id - ID_RAIL) % 4u;
        var uv = vec2<f32>(dw, -g.y);
        if face == FACE_TOP { uv = vec2<f32>(dw, g.x); } else if face == FACE_FRONT { uv = vec2<f32>(g.x, -g.y); }
        var tile = W.deck_tile;
        var tex = 7.0;
        if m == RAIL_STONE {
            if W.walls_on != 0u { tile = W.wall_tile; tex = 2.0; } else { tile = W.path_tile; tex = 0.0; }
        } else if m == RAIL_WOOD { tex = 4.0; }
        return vec3<f32>(div(uv.x, tile), div(uv.y, tile), tex);
    }
    if id == ID_GROUND || id == ID_RISER {
        var along = dw;
        if id == ID_RISER { along = dw + g.y; }
        let on_path = W.verge_on == 0u || abs(g.x) < path_edge(g.d) || on_fork(g.x, g.d);
        if on_path { return vec3<f32>(rot(1u, div(g.x, W.path_tile), div(along, W.path_tile)), 0.0); }
        return vec3<f32>(rot(2u, div(g.x, W.verge_tile), div(along, W.verge_tile)), 1.0);
    }
    if id == ID_WALL_L || id == ID_WALL_R {
        return vec3<f32>(rot(4u, div(dw, W.wall_tile), div(-g.y, W.wall_tile)), 2.0);
    }
    if id == ID_CEILING {
        return vec3<f32>(rot(8u, div(g.x, W.ceil_tile), div(dw, W.ceil_tile)), 3.0);
    }
    return vec3<f32>(0.0, 0.0, -1.0);
}

@compute @workgroup_size(8, 8)
fn uvs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= W.width || id.y >= W.height { return; }
    let i = id.y * W.width + id.x;
    let s = surface_uv(gbuf[i]);
    uvs[i * 3u] = s.x; uvs[i * 3u + 1u] = s.y; uvs[i * 3u + 2u] = s.z;
}

// render::gloss_of: (gloss, ripples).
fn gloss_of(tex: u32, gid: u32) -> vec2<f32> {
    if gid != ID_GROUND && gid != ID_CHASM { return vec2<f32>(0.0); }
    var g = 0.0; var r = 0.0;
    if tex == 0u { g = W.path_gloss; r = W.path_rip; }
    else if tex == 1u { g = W.verge_gloss; r = W.verge_rip; }
    else if tex == 4u { g = W.deck_gloss; r = W.deck_rip; }
    else if tex == 5u {
        if W.br_bottom == 1u { return vec2<f32>(1.0, 0.3); }
        if W.verge_on != 0u { g = W.verge_gloss; r = W.verge_rip; } else { g = W.path_gloss; r = W.path_rip; }
    } else { return vec2<f32>(0.0); }
    return vec2<f32>(clamp(g, 0.0, 1.0), clamp(r, 0.0, 1.0));
}

// render::ripple_slope (WAVES: wavelength, heading, phase, cycles per loop).
fn ripple_slope(x: f32, w: f32, ripples: f32, footprint: f32) -> vec2<f32> {
    if ripples <= 0.0 { return vec2<f32>(0.0); }
    let waves = array<vec4<f32>, 4>(vec4<f32>(1.9, 0.5, 0.0, 2.0), vec4<f32>(1.15, -0.8, 1.3, 3.0), vec4<f32>(0.62, 0.2, 2.1, 5.0), vec4<f32>(0.37, 1.1, 0.7, 7.0));
    let t = W.tphase * TAU;
    var s2 = vec2<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let wv = waves[k];
        let fade = clamp((wv.x / max(footprint, 1e-3) - 3.0) / 6.0, 0.0, 1.0);
        if fade <= 0.0 { continue; }
        let kk = TAU / wv.x;
        let dx = cos(wv.y); let dz = sin(wv.y);
        let s = 0.13 * ripples * fade * cos(kk * (x * dx + w * dz) + wv.z - wv.w * t);
        s2 += vec2<f32>(s * dx, s * dz);
    }
    return s2;
}

// weather::rain_rings
fn rain_rings(x: f32, w: f32) -> vec2<f32> {
    let cell = snap_to_loop(0.42, W.loop_len);
    let n = i32(rround(div(W.loop_len, cell)));
    let cr = cycles(1.4, W.loop_seconds);
    let seed = W.precip_seed ^ 0x5151u;
    let cx = i32(floor(div(x, cell))); let cw = i32(floor(div(w, cell)));
    var sl = vec2<f32>(0.0);
    for (var ox = -1; ox <= 1; ox++) {
        for (var ow = -1; ow <= 1; ow++) {
            let ix = cx + ox; let iw = cw + ow;
            let s = seed ^ (bitcast<u32>(ix) * 0x85EBCA6Bu);
            let tt = hf(s ^ 0x31u, rem_ei(iw, n)) + cr * W.tphase;
            let k = rem_ei(i32(floor(tt)), i32(cr));
            let a = tt - floor(tt);
            let key = rem_ei(iw, n) * 1031 + k;
            if hf(s ^ 0x32u, key) > 0.35 + 0.6 * W.wx_rings { continue; }
            let px = (f32(ix) + hf(s ^ 0x33u, key)) * cell;
            let pw = (f32(iw) + hf(s ^ 0x34u, key)) * cell;
            let dx = x - px; let dw = w - pw;
            let r = sqrt(dx * dx + dw * dw);
            let rad = 0.02 + 0.22 * a;
            if abs(r - rad) > 0.093 { continue; }
            let q = (r - rad) / 0.035;
            let env = (1.0 - a) * (1.0 - a) * exp(-(q * q));
            if env < 1e-3 { continue; }
            let slope = 0.25 * env * sin(TAU * (r - rad) / 0.045);
            let rr = max(r, 1e-4);
            sl += vec2<f32>(slope * dx / rr, slope * dw / rr);
        }
    }
    return sl;
}

struct Surf { albedo: vec3<f32>, gloss: f32, ripples: f32, rings: vec2<f32> }

// weather::surface
fn weather_surface(g: GPix, id: u32, tex: u32, albedo: vec3<f32>, gloss: f32, ripples: f32) -> Surf {
    var o = Surf(albedo, gloss, ripples, vec2<f32>(0.0));
    let floor_px = (id == ID_GROUND || id == ID_RISER) && (tex == 0u || tex == 1u || tex == 4u);
    let w = W.scroll + g.d;
    let seed = W.precip_seed;
    if W.wx_wet > 0.0 {
        if floor_px {
            o.albedo = o.albedo * (1.0 - select(0.4, 0.25, tex == 1u) * W.wx_wet);
            o.gloss = max(o.gloss, 0.32 * W.wx_wet);
            o.ripples = max(o.ripples, 0.35 * W.wx_wet);
            if W.wx_puddles > 0.0 && id == ID_GROUND {
                let n = 0.65 * noise2(g.x, w, 1.6, W.loop_len, seed ^ 0x9Du) + 0.35 * noise2(g.x, w, 0.55, W.loop_len, seed ^ 0x9Eu);
                let th = 1.0 - 0.75 * W.wx_puddles;
                let pm = smoothstep_r(th - 0.03, th + 0.03, n);
                if pm > 0.0 {
                    o.albedo = o.albedo * (1.0 - 0.45 * pm);
                    o.gloss += (0.9 - o.gloss) * pm;
                    o.ripples += (0.04 - o.ripples) * pm;
                    if W.wx_rings > 0.0 { o.rings = rain_rings(g.x, w) * pm; }
                }
            }
        } else if id == ID_WALL_L || id == ID_WALL_R || id == ID_FACADE || id == ID_CLIFF || is_rail(id) {
            o.albedo = o.albedo * (1.0 - 0.18 * W.wx_wet);
        }
    }
    if W.wx_snow > 0.0 {
        let snow = vec3<f32>(0.80, 0.83, 0.88);
        let s = W.wx_snow;
        if floor_px && id == ID_GROUND {
            let n = 0.55 * noise2(g.x, w, 0.5, W.loop_len, seed ^ 0xA1u) + 0.3 * noise2(g.x, w, 0.17, W.loop_len, seed ^ 0xA2u) + 0.15 * noise2(g.x, w, 1.6, W.loop_len, seed ^ 0xA4u);
            let th = 1.1 - 1.2 * s;
            var cov = smoothstep_r(th - 0.22, th + 0.22, n);
            if tex != 1u && W.wx_track > 0.0 {
                let edge = max(path_half_width(g.d), 0.1);
                let t = W.wx_track * (1.0 - smoothstep_r(0.3, 0.75, abs(g.x) / edge));
                let slush = mix3(albedo * 0.7, vec3<f32>(0.42, 0.43, 0.46), 0.55);
                o.albedo = mix3(o.albedo, slush, t * min(s, 1.0) * 0.8);
                cov *= 1.0 - 0.85 * t;
            }
            o.albedo = mix3(o.albedo, snow, cov);
            o.gloss *= 1.0 - cov;
            o.ripples *= 1.0 - cov;
        } else if id == ID_RISER {
            o.albedo = mix3(o.albedo, snow, 0.35 * s);
        } else if is_rail(id) && (id - ID_RAIL) % 4u == FACE_TOP {
            o.albedo = mix3(o.albedo, snow, min(1.6 * s, 0.95));
        } else if id == ID_WALL_L || id == ID_WALL_R {
            let top = W.wall_top;
            let cap = 0.04 + 0.08 * s;
            let drift = 0.35 * s * (0.5 + noise2(0.0, w, 0.9, W.loop_len, seed ^ 0xA3u));
            var on_top = 0.0;
            if top < 1.0e30 { on_top = smoothstep_r(top - cap - 0.02, top - cap, g.y); }
            let at_foot = 1.0 - smoothstep_r(drift - 0.03, drift, g.y);
            o.albedo = mix3(o.albedo, snow, max(on_top, at_foot));
        }
    }
    return o;
}

// Ctx::sky_visibility
fn sky_visibility(x: f32, y: f32, d: f32, dir: vec3<f32>, skip: u32) -> f32 {
    if W.ceiling_on != 0u { return 0.0; }
    if W.walls_on == 0u { return 1.0; }
    let xw = wall_x(d);
    let top = W.wall_top;
    if dir.x > 1e-4 && skip != ID_WALL_R {
        let t = (xw - x) / dir.x;
        if t > 0.0 && y + dir.y * t < top { return 0.0; }
    }
    if dir.x < -1e-4 && skip != ID_WALL_L {
        let t = (-xw - x) / dir.x;
        if t > 0.0 && y + dir.y * t < top { return 0.0; }
    }
    return 1.0;
}

fn light_pos(k: u32) -> vec4<f32> { return vec4<f32>(lights[k * 8u], lights[k * 8u + 1u], lights[k * 8u + 2u], lights[k * 8u + 3u]); }
fn light_col(k: u32) -> vec3<f32> { return vec3<f32>(lights[k * 8u + 4u], lights[k * 8u + 5u], lights[k * 8u + 6u]); }
fn sky_dir(k: u32) -> vec3<f32> { let b = (W.n_lights + k) * 8u; return vec3<f32>(lights[b], lights[b + 1u], lights[b + 2u]); }
fn sky_col(k: u32) -> vec3<f32> { let b = (W.n_lights + k) * 8u; return vec3<f32>(lights[b + 4u], lights[b + 5u], lights[b + 6u]); }

// The point lights this pixel looks at: the tile's list, or all of them.
fn tile_range(px: u32, py: u32) -> vec2<u32> {
    if W.tiles_on == 0u { return vec2<u32>(0u, W.n_lights); }
    let t = (py / 16u) * W.tile_cols + px / 16u;
    return vec2<u32>(tiles[t], tiles[t + 1u]);
}
fn light_index(k: u32) -> u32 {
    if W.tiles_on == 0u { return k; }
    let ntiles = W.tile_cols * ((W.height + 15u) / 16u);
    return tiles[ntiles + 1u + k];
}

// Ctx::light_among for a surface with normal n (the portal is a crossing's, not one world's).
fn light_at(x: f32, y: f32, d: f32, n: vec3<f32>, skip: u32, sun_mask: f32, px: u32, py: u32) -> vec3<f32> {
    var l = vec3<f32>(W.amb_r, W.amb_g, W.amb_b);
    for (var k = 0u; k < W.n_sky; k++) {
        let dir = sky_dir(k);
        let ndl = max(dot(n, dir), 0.0);
        if ndl <= 0.0 { continue; }
        let vis = sky_visibility(x, y, d, dir, skip) * sun_mask;
        if vis > 0.0 { l += sky_col(k) * (ndl * vis); }
    }
    if W.n_lights > 0u {
        let p = to_cam(x, y, d);
        let r = tile_range(px, py);
        for (var k = r.x; k < r.y; k++) {
            let li = light_index(k);
            let lp = light_pos(li);
            let v = lp.xyz - p;
            let d2 = dot(v, v);
            let r2 = lp.w * lp.w;
            if d2 >= r2 { continue; }
            let wgt = 1.0 - d2 / r2;
            let att = wgt * wgt / (1.0 + 0.3 * d2);
            let ndl = max(dot(n, v) / max(sqrt(d2), 1e-4), 0.0) * 0.85 + 0.15;
            l += light_col(li) * (att * ndl);
        }
    }
    if W.bands >= 2u {
        let lum = 0.3 * l.x + 0.55 * l.y + 0.15 * l.z;
        if lum > 1e-5 {
            let b = f32(W.bands) * 0.6;
            let q = rround(lum * b) / b;
            l = l * (q / lum);
        }
    }
    return l;
}

fn fres(c: f32) -> f32 { let m = max(1.0 - c, 0.0); return 0.02 + 0.98 * m * m * m * m * m; }

@compute @workgroup_size(8, 8)
fn shade_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height { return; }
    let w = W.width; let h = W.height;
    let x = gid.x; let y = gid.y;
    let i = y * w + x;
    let g = gbuf[i];
    let id = g.idr & 0xFFu;
    let realm = g.idr >> 8u;
    for (var k = 0u; k < 6u; k++) { refl[i * 6u + k] = 0.0; }
    let texf = uvs[i * 3u + 2u];
    if texf < 0.0 {
        let c = sky_base(f32(y));
        hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
        return;
    }
    let u = uvs[i * 3u]; let v = uvs[i * 3u + 1u];
    let tex = u32(texf);
    // Footprint from neighbouring pixels on the same surface, like GPU derivatives.
    var fp = 0.0;
    var js = array<u32, 2>(select(i - 1u, i + 1u, x + 1u < w), select(i - w, i + w, y + 1u < h));
    for (var k = 0; k < 2; k++) {
        let j = js[k];
        let gj = gbuf[j];
        if (gj.idr & 0xFFu) == id && (gj.idr >> 8u) == realm {
            let t2 = uvs[j * 3u + 2u];
            if t2 >= 0.0 && u32(t2) == tex { fp = max(fp, max(abs(uvs[j * 3u] - u), abs(uvs[j * 3u + 1u] - v))); }
        }
    }
    let slot = select(tex, 4u, tex == 7u);
    var albedo = tex_sample(slot, u, v, min(fp, 4.0));
    if tex == 7u {
        let lw = vec3<f32>(0.3, 0.59, 0.11);
        let grain = clamp(dot(albedo, lw) / max(dot(vec3<f32>(W.deck_base_r, W.deck_base_g, W.deck_base_b), lw), 1e-3), 0.6, 1.3);
        albedo = vec3<f32>(W.rail_r, W.rail_g, W.rail_b) * grain;
    }
    var ao = masks[w * h + i];
    var normal = vec3<f32>(0.0, -1.0, 0.0);
    var skip = ID_CEILING;
    if id == ID_GROUND {
        ao *= passage_dark(g.x, g.d);
        if tex == 0u && W.fork_on != 0u && W.fk_style != 0u && W.walls_on == 0u {
            ao *= split_edge_dark(g.x, g.d);
        } else if tex == 0u && !on_fork(g.x, g.d) {
            let edge = max(path_edge(g.d), 0.05);
            ao *= 1.0 - clamp(W.edge_dark, 0.0, 1.0) * smoothstep_r(0.5, 1.0, abs(g.x) / edge);
        }
        if W.walls_on != 0u {
            let dist = max(wall_x(g.d) - abs(g.x), 0.0);
            ao *= 1.0 - clamp(W.base_shadow, 0.0, 1.0) * exp(-dist / 0.45);
        }
        if W.stairs_on != 0u {
            let at = W.scroll + g.d;
            var px_d = 0.0;
            if i >= w && (gbuf[i - w].idr & 0xFFu) == ID_GROUND { px_d = max(px_d, abs(gbuf[i - w].d - g.d)); }
            if i + w < w * h && (gbuf[i + w].idr & 0xFFu) == ID_GROUND { px_d = max(px_d, abs(gbuf[i + w].d - g.d)); }
            let nose = select(st_to_next_riser(at), st_since_riser(at), W.st_rise > 0.0);
            let nw = max(0.035, px_d);
            ao *= 1.0 + 0.35 * (0.035 / nw) * smoothstep_r(nw, 0.0, nose);
            if W.st_rise > 0.0 {
                let cw = max(0.06, px_d);
                ao *= 1.0 - 0.45 * (0.06 / cw) * exp(-st_to_next_riser(at) / cw);
            }
        }
        normal = vec3<f32>(0.0, 1.0, 0.0); skip = 0u;
    } else if id == ID_RISER {
        if tex == 0u {
            let edge = max(path_edge(g.d), 0.05);
            ao *= 1.0 - clamp(W.edge_dark, 0.0, 1.0) * smoothstep_r(0.5, 1.0, abs(g.x) / edge);
        }
        ao *= 0.9 * (1.0 - 0.35 * smoothstep_r(-0.03, 0.0, g.y));
        let up = W.stairs_on == 0u || W.st_rise > 0.0;
        normal = vec3<f32>(0.0, 0.0, select(1.0, -1.0, up)); skip = 0u;
    } else if id == ID_CHASM {
        ao *= 0.25 + 0.75 * exp(-max(W.br_floor, 0.0) / 8.0);
        if W.br_bottom == 0u { albedo = albedo * vec3<f32>(W.bottom_r, W.bottom_g, W.bottom_b); }
        normal = vec3<f32>(0.0, 1.0, 0.0); skip = 0u;
    } else if id == ID_CLIFF {
        ao *= 0.85 * passage_dark(g.x, g.d);
        normal = vec3<f32>(0.0, 0.0, -1.0); skip = 0u;
    } else if is_rail(id) {
        let out = select(-1.0, 1.0, g.x > 0.0);
        ao *= 1.0 - 0.3 * exp(-max(g.y, 0.0) / 0.25);
        let face = (id - ID_RAIL) % 4u;
        if face == FACE_INNER { normal = vec3<f32>(-out, 0.0, 0.0); }
        else if face == FACE_OUTER { normal = vec3<f32>(out, 0.0, 0.0); }
        else if face == FACE_TOP { normal = vec3<f32>(0.0, 1.0, 0.0); }
        else { normal = vec3<f32>(0.0, 0.0, -1.0); }
        skip = 0u;
    } else if id == ID_WALL_L {
        ao *= 1.0 - clamp(W.base_shadow, 0.0, 1.0) * exp(-g.y / 0.5);
        normal = vec3<f32>(1.0, 0.0, 0.0); skip = ID_WALL_L;
    } else if id == ID_WALL_R {
        ao *= 1.0 - clamp(W.base_shadow, 0.0, 1.0) * exp(-g.y / 0.5);
        normal = vec3<f32>(-1.0, 0.0, 0.0); skip = ID_WALL_R;
    } else {
        ao *= passage_dark(g.x, g.d);
    }
    let gr = gloss_of(tex, id);
    let s = weather_surface(g, id, tex, albedo, gr.x, gr.y);
    let sun = masks[i];
    let light = light_at(g.x, g.y, g.d, normal, skip, sun, x, y);
    var c = s.albedo * (light * ao);
    if s.gloss > 0.001 {
        let gloss = s.gloss;
        let p = to_cam(g.x, g.y, g.d);
        let plen = max(length(p), 1e-4);
        let vv = -p / plen;
        let eye = max(-p.y, 0.05);
        let footprint = max(p.z * p.z / (W.focal_px * eye), p.z / W.focal_px);
        let rs = ripple_slope(g.x, W.scroll + g.d, s.ripples, footprint) + s.rings;
        let n = normalize(vec3<f32>(-rs.x, 1.0, -rs.y));
        let ndv = max(dot(n, vv), 1e-3);
        let r = gloss * fres(ndv);
        let rough = min(0.04 + 0.45 * (1.0 - gloss) + 0.12 * s.ripples, 0.7);
        let rv = 2.0 * ndv * n - vv;
        var env: vec3<f32>;
        let hy = max(W.horizon_px, 1.0);
        if W.sky_on != 0u {
            var row_at = 0.0;
            if rv.z > 1e-3 { row_at = hy - W.focal_px * max(rv.y, 0.0) / rv.z; }
            let st = vec3<f32>(W.sky_top_r, W.sky_top_g, W.sky_top_b);
            let sh = vec3<f32>(W.sky_hor_r, W.sky_hor_g, W.sky_hor_b);
            env = mix3(st, sh, pow(clamp((row_at - W.top) / max(hy - W.top, 1.0), 0.0, 1.0), 1.3));
        } else {
            env = vec3<f32>(W.amb_r, W.amb_g, W.amb_b) * 0.3;
            if W.env_fog != 0u { env += fog_col() * 0.5; }
        }
        let e = clamp(2.0 / (rough * rough) - 2.0, 4.0, 600.0);
        let norm = (e + 8.0) / (8.0 * PI);
        var spec = vec3<f32>(0.0);
        let rng = tile_range(x, y);
        for (var k = rng.x; k < rng.y; k++) {
            let li = light_index(k);
            let lp = light_pos(li);
            let d3 = lp.xyz - p;
            let d2 = dot(d3, d3);
            let r2 = lp.w * lp.w;
            if d2 >= r2 { continue; }
            let wgt = 1.0 - d2 / r2;
            let dist = max(sqrt(d2), 1e-4);
            let l = d3 / dist;
            let ndl = dot(n, l);
            if ndl <= 0.0 { continue; }
            let hv = (l + vv) / max(length(l + vv), 1e-5);
            spec += light_col(li) * (norm * pow(max(dot(n, hv), 0.0), e) * fres(max(dot(hv, vv), 0.0)) * ndl * gloss * (wgt * wgt / (1.0 + 0.3 * d2)));
        }
        for (var k = 0u; k < W.n_sky; k++) {
            let l = sky_dir(k);
            let ndl = dot(n, l);
            if ndl <= 0.0 { continue; }
            let hv = (l + vv) / max(length(l + vv), 1e-5);
            spec += sky_col(k) * (norm * pow(max(dot(n, hv), 0.0), e) * fres(max(dot(hv, vv), 0.0)) * ndl * gloss * sun);
        }
        c = c * (1.0 - r) + env * r + spec;
        refl[i * 6u] = r * fog_t(g.depth);
        refl[i * 6u + 1u] = env.x * W.gain; refl[i * 6u + 2u] = env.y * W.gain; refl[i * 6u + 3u] = env.z * W.gain;
        refl[i * 6u + 4u] = rough;
        refl[i * 6u + 5u] = rs.x;
    }
    let o = apply_fog(c, g.depth);
    hdr[i * 3u] = o.x; hdr[i * 3u + 1u] = o.y; hdr[i * 3u + 2u] = o.z;
}

// ── Mist (weather::apply_mist) ─────────────────────────────────────────────

// weather::drifting_noise
fn drifting_noise(x: f32, w: f32, cell: f32, travel: f32, seed: u32) -> f32 {
    var n = 0.0;
    for (var j = 0u; j < 2u; j++) {
        let ph = rem_e(W.tphase + f32(j) * 0.5, 1.0);
        let sn = sin(PI * ph);
        let wt = sn * sn;
        let xo = x - rnd(travel * (ph - 0.5)) + f32(j) * 53.0;
        n += wt * (0.7 * noise2(xo, w, cell, W.loop_len, seed ^ 0x51u ^ j) + 0.3 * noise2(xo, w, rnd(cell * 0.37), W.loop_len, seed ^ 0x53u ^ j));
    }
    return n;
}

// weather::mist_patch
fn mist_patch(x: f32, d: f32) -> f32 {
    let p = clamp(A.patchy, 0.0, 1.0);
    if p <= 0.0 { return 1.0; }
    let n = drifting_noise(x, W.scroll + d, 5.0, A.travel, A.seed ^ 0xB1u);
    return max(1.0 - p + p * 2.0 * smoothstep_r(0.2, 0.8, n), 0.0);
}

fn sky_air() -> vec3<f32> {
    var l = vec3<f32>(W.amb_r, W.amb_g, W.amb_b);
    for (var k = 0u; k < W.n_sky; k++) { l += sky_col(k) * 0.5; }
    return l;
}

// weather::air_light (light_among with no normal), every lamp in index order.
fn air_light(x: f32, y: f32, d: f32) -> vec3<f32> {
    if d >= 18.0 { return sky_air(); }
    var l = vec3<f32>(W.amb_r, W.amb_g, W.amb_b);
    for (var k = 0u; k < W.n_sky; k++) {
        let dir = sky_dir(k);
        let ndl = clamp(0.45 - 0.35 * dir.z, 0.15, 0.8);
        let vis = sky_visibility(x, y, d, dir, 0u);
        if vis > 0.0 { l += sky_col(k) * (ndl * vis); }
    }
    if W.n_lights > 0u {
        let p = to_cam(x, y, d);
        for (var li = 0u; li < W.n_lights; li++) {
            let lp = light_pos(li);
            let v = lp.xyz - p;
            let d2 = dot(v, v);
            let r2 = lp.w * lp.w;
            if d2 >= r2 { continue; }
            let wgt = 1.0 - d2 / r2;
            let att = wgt * wgt / (1.0 + 0.3 * d2);
            l += light_col(li) * (att * 0.8);
        }
    }
    if W.bands >= 2u {
        let lum = 0.3 * l.x + 0.55 * l.y + 0.15 * l.z;
        if lum > 1e-5 {
            let b = f32(W.bands) * 0.6;
            let q = rround(lum * b) / b;
            l = l * (q / lum);
        }
    }
    return l;
}

@compute @workgroup_size(8, 8)
fn mist_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= W.width || gid.y >= W.height || A.mist_on == 0u { return; }
    let x = gid.x; let row = gid.y;
    let i = row * W.width + x;
    let g = gbuf[i];
    let none = (g.idr & 0xFFu) == ID_NONE;
    let eye = W.eye_height;
    let top = A.top;
    let slope = (W.horizon_px - f32(row) - 0.5) / W.focal_px;
    let across = (f32(x) + 0.5 - W.center_px) / W.focal_px;
    let stretch = sqrt(1.0 + slope * slope + across * across);
    var len: f32; var z: f32; var gx: f32; var gd: f32;
    if none {
        if eye >= top { return; }
        z = W.far;
        if slope > 1e-3 { z = min((top - eye) / slope, W.far); }
        len = z * stretch; gx = 0.0; gd = z;
    } else {
        z = min(g.depth, W.far * 2.0);
        let y_seen = eye + slope * z;
        let lo = min(eye, y_seen); let hi = max(eye, y_seen);
        var inside = 0.0;
        if hi <= top { inside = 1.0; } else if lo < top { inside = (top - lo) / max(hi - lo, 1e-4); }
        len = z * stretch * inside; gx = g.x; gd = g.d;
    }
    if len <= 1e-3 { return; }
    let tau = max(A.density, 0.0) * mist_patch(gx, gd) * len;
    let a = 1.0 - exp(-tau);
    if a < 0.003 { return; }
    var light: vec3<f32>;
    if none { light = sky_air(); } else { light = air_light(gx, min(top * 0.5, eye), gd); }
    let col = apply_fog(vec3<f32>(A.mist_r, A.mist_g, A.mist_b) * light, z * 0.5);
    let c = mix3(vec3<f32>(hdr[i * 3u], hdr[i * 3u + 1u], hdr[i * 3u + 2u]), col, a);
    hdr[i * 3u] = c.x; hdr[i * 3u + 1u] = c.y; hdr[i * 3u + 2u] = c.z;
}
