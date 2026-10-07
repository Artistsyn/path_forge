//! Tools for looking at renders: downscaling, contact sheets, loop checks, PNG encoding.
//! Shared by the `pf` CLI and the MCP server.

use crate::scene::Scene;
use crate::world::{Image, RenderOptions, WorldRenderer};
use serde::Serialize;

/// Box-filtered downscale by `scale` (0..1].
pub fn downscale(img: &Image, scale: f32) -> Image {
    let scale = scale.clamp(0.02, 1.0);
    if scale >= 0.999 { return Image { width: img.width, height: img.height, rgba: img.rgba.clone(), stats: None, depth: None, pick: None }; }
    let (w, h) = (img.width, img.height);
    let (tw, th) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    let mut out = vec![255u8; tw * th * 4];
    for y in 0..th {
        let (y0, y1) = (y * h / th, ((y + 1) * h / th).max(y * h / th + 1).min(h));
        for x in 0..tw {
            let (x0, x1) = (x * w / tw, ((x + 1) * w / tw).max(x * w / tw + 1).min(w));
            let mut acc = [0u32; 4];
            let mut n = 0;
            for sy in y0..y1 { for sx in x0..x1 {
                let i = (sy * w + sx) * 4;
                for c in 0..4 { acc[c] += img.rgba[i + c] as u32; }
                n += 1;
            }}
            let o = (y * tw + x) * 4;
            for c in 0..4 { out[o + c] = (acc[c] / n.max(1)) as u8; }
        }
    }
    Image { width: tw, height: th, rgba: out, stats: None, depth: None, pick: None }
}

/// Lay images out in a grid on a dark background.
pub fn sheet(images: &[Image], columns: usize) -> Image {
    let cols = columns.clamp(1, images.len().max(1));
    let rows = images.len().div_ceil(cols).max(1);
    let (tw, th) = images.iter().fold((0, 0), |(a, b), i| (a.max(i.width), b.max(i.height)));
    let gap = 4;
    let (sw, sh) = (cols * tw + (cols + 1) * gap, rows * th + (rows + 1) * gap);
    let mut rgba = vec![0u8; sw * sh * 4];
    for px in rgba.chunks_exact_mut(4) { px.copy_from_slice(&[40, 40, 48, 255]); }
    for (k, img) in images.iter().enumerate() {
        let (ox, oy) = (gap + (k % cols) * (tw + gap), gap + (k / cols) * (th + gap));
        for y in 0..img.height {
            let d = ((oy + y) * sw + ox) * 4;
            rgba[d..d + img.width * 4].copy_from_slice(&img.rgba[y * img.width * 4..(y + 1) * img.width * 4]);
        }
    }
    Image { width: sw, height: sh, rgba, stats: None, depth: None, pick: None }
}

pub fn png_bytes(img: &Image) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&img.rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Mean absolute RGB difference (0..255) and the share of pixels whose largest channel differs by more than `thresh`.
pub fn diff(a: &[u8], b: &[u8], thresh: u8) -> (f64, f64) {
    let mut sum = 0u64;
    let mut over = 0u64;
    let n = (a.len() / 4).max(1);
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut m = 0u8;
        for c in 0..3 {
            let d = pa[c].abs_diff(pb[c]);
            sum += d as u64;
            m = m.max(d);
        }
        if m > thresh { over += 1; }
    }
    (sum as f64 / (n as f64 * 3.0), over as f64 / n as f64)
}

#[derive(Clone, Debug, Serialize)]
pub struct LoopReport {
    /// Difference between the frame at the loop length and the first frame; ~0 means seamless.
    pub exact: f64,
    /// Mean difference between consecutive frames (normal motion).
    pub step: f64,
    /// Difference between the last frame and the first (what the loop point looks like).
    pub wrap: f64,
    pub wrap_over_step: f64,
    pub largest_step: f64,
    pub largest_step_at: usize,
    pub seamless: bool,
    pub verdict: String,
}

pub fn check_loop(r: &mut WorldRenderer, scene: &Scene, opts: &RenderOptions, frames: usize) -> LoopReport {
    let frames = frames.clamp(4, 96);
    let len = scene.motion.loop_length.max(1.0);
    let mut prev: Option<Vec<u8>> = None;
    let mut first: Option<Vec<u8>> = None;
    let mut steps = Vec::new();
    for i in 0..frames {
        let f = r.render(scene, len * i as f32 / frames as f32, opts).rgba;
        if let Some(p) = &prev { steps.push(diff(p, &f, 8).0); }
        if first.is_none() { first = Some(f.clone()); }
        prev = Some(f);
    }
    let (first, last) = (first.unwrap(), prev.unwrap());
    let end = r.render(scene, len, opts).rgba;
    let (exact, exact_px) = diff(&end, &first, 8);
    let wrap = diff(&last, &first, 8).0;
    let step = steps.iter().sum::<f64>() / steps.len().max(1) as f64;
    let (largest_step_at, largest_step) = steps.iter().copied().enumerate().fold((0, 0.0), |a, (i, d)| if d > a.1 { (i, d) } else { a });
    let ratio = if step > 1e-9 { wrap / step } else { 0.0 };
    let seamless = exact < 0.05 && exact_px < 0.0005;
    let verdict = if !seamless {
        format!("jumps at the loop point: the last frame does not lead back into the first (difference {exact:.2})")
    } else if ratio > 2.0 {
        format!("loops exactly, but the step into the loop point is {ratio:.1}x a normal step: something pops just before the wrap")
    } else {
        "seamless: the loop point looks like any other frame step".into()
    };
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    LoopReport { exact: r3(exact), step: r3(step), wrap: r3(wrap), wrap_over_step: r3(ratio), largest_step: r3(largest_step), largest_step_at, seamless, verdict }
}

/// Every frame of a GIF as full RGBA images, composited the way a viewer shows them.
pub fn decode_gif(path: &std::path::Path) -> Result<Vec<Image>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(file).map_err(|e| e.to_string())?;
    let (w, h) = (dec.width() as usize, dec.height() as usize);
    let mut canvas = vec![0u8; w * h * 4];
    let mut out = Vec::new();
    while let Some(f) = dec.read_next_frame().map_err(|e| e.to_string())? {
        let before = canvas.clone();
        let (fl, ft, fw, fh) = (f.left as usize, f.top as usize, f.width as usize, f.height as usize);
        for y in 0..fh {
            for x in 0..fw {
                let (cx, cy) = (fl + x, ft + y);
                if cx >= w || cy >= h { continue; }
                let s = (y * fw + x) * 4;
                if f.buffer[s + 3] == 0 { continue; }
                canvas[(cy * w + cx) * 4..(cy * w + cx) * 4 + 4].copy_from_slice(&f.buffer[s..s + 4]);
            }
        }
        out.push(Image { width: w, height: h, rgba: canvas.clone(), stats: None, depth: None, pick: None });
        match f.dispose {
            gif::DisposalMethod::Background => for y in ft..(ft + fh).min(h) { for x in fl..(fl + fw).min(w) { canvas[(y * w + x) * 4..(y * w + x) * 4 + 4].fill(0); } },
            gif::DisposalMethod::Previous => canvas = before,
            _ => {}
        }
    }
    Ok(out)
}

/// The absolute difference of two images, amplified, for showing where they differ.
pub fn diff_image(a: &Image, b: &Image, gain: f32) -> Image {
    let rgba = a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)).flat_map(|(p, q)| {
        let d = |k: usize| ((p[k].abs_diff(q[k]) as f32) * gain).min(255.0) as u8;
        [d(0), d(1), d(2), 255]
    }).collect();
    Image { width: a.width, height: a.height, rgba, stats: None, depth: None, pick: None }
}

/// Every frame of an animated WebP as full RGBA images.
pub fn decode_webp(path: &std::path::Path) -> Result<Vec<Image>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dec = webp_animation::Decoder::new(&bytes).map_err(|e| format!("webp: {e:?}"))?;
    let (w, h) = dec.dimensions();
    Ok(dec.into_iter().map(|f| Image { width: w as usize, height: h as usize, rgba: f.data().to_vec(), stats: None, depth: None, pick: None }).collect())
}

/// Frames of a GIF or animated WebP, chosen by extension.
pub fn decode_animation(path: &std::path::Path) -> Result<Vec<Image>, String> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("gif") => decode_gif(path),
        Some("webp") => decode_webp(path),
        _ => Err(format!("{}: expected a .gif or .webp", path.display())),
    }
}

/// How much of the frame one kind of surface covers and how bright it is there.
#[derive(Clone, Copy, Debug, Default)]
pub struct Surface {
    /// Average share of the frame, 0..1.
    pub coverage: f32,
    /// Mean luma where it shows, 0..255.
    pub luma: f32,
    /// Largest share in any one frame.
    pub peak: f32,
}

/// Surfaces from one frame's statistics.
pub fn surfaces(stats: &crate::world::render::FrameStats) -> std::collections::BTreeMap<String, Surface> {
    stats.coverage.iter().map(|(k, c)| (k.clone(), Surface { coverage: *c, luma: stats.luma.get(k).copied().unwrap_or(0.0), peak: *c })).collect()
}

/// Things that usually look wrong, judged from what the frames show.
pub fn stats_warnings(scene: &Scene, mean: f32, surf: &std::collections::BTreeMap<String, Surface>) -> Vec<String> {
    let mut warnings = Vec::new();
    let get = |k: &str| surf.get(k).copied().unwrap_or_default();
    if mean < 30.0 { warnings.push(format!("very dark overall (mean luma {mean:.0}/255): raise post.exposure, light.ambient or fixture intensity")); }
    if mean > 200.0 { warnings.push(format!("washed out (mean luma {mean:.0}/255): lower post.exposure or light.ambient")); }
    let path = get("path");
    if path.coverage < 0.05 { warnings.push(format!("the path covers only {:.1}% of the frame: widen path.half_width, lower camera.horizon or camera.eye_height", path.coverage * 100.0)); }
    if path.luma < 25.0 && path.coverage > 0.0 { warnings.push(format!("the path is very dark (luma {:.0}): players need to read it; add light near the camera or raise ambient", path.luma)); }
    let verge = get("verge");
    if verge.coverage > 0.05 && path.coverage > 0.05 && (verge.luma - path.luma).abs() < 8.0 && scene.path.material.pattern != scene.verge.material.pattern {
        warnings.push(format!("the path (luma {:.0}) and verge (luma {:.0}) are nearly the same brightness, so the path edge may not read; change one material's colour or brightness", path.luma, verge.luma));
    }
    let active_props = scene.props.iter().filter(|p| p.enabled && p.density > 0.0).count();
    if active_props > 0 && get("props").peak < 0.002 { warnings.push("props are configured but almost never on screen: check their lateral (they may be behind walls or off-screen), spacing and scale".into()); }
    let active_fx = scene.fixtures.iter().filter(|f| f.enabled).count();
    if active_fx > 0 && get("fixtures").peak < 0.0005 { warnings.push("fixtures are configured but not visible: check mount, height, lateral and size".into()); }
    for (k, s) in surf {
        if s.coverage > 0.03 && s.luma > 235.0 { warnings.push(format!("{k} are blown out (mean luma {:.0}): lower the lights near them, post.exposure or post.bloom", s.luma)); }
    }
    if scene.sky.enabled && get("sky").peak < 0.002 { warnings.push("sky is enabled but hidden (walls or ceiling cover it): disable it to save work, or lower the walls".into()); }
    warnings
}

/// Every prop of a kit (`@name` for a built-in one, or a kit folder) standing beside the path of
/// `backdrop`, one rendered frame each, in name order. Returns the sheet, the names in order with
/// their descriptions, and any problems found resolving them.
pub fn kit_sheet(r: &mut WorldRenderer, kit: &str, only: Option<&str>, backdrop: &Scene, base: Option<std::path::PathBuf>, scale: f32, columns: usize)
    -> Result<(Image, Vec<(String, String)>, Vec<String>), String> {
    use crate::scene::{PropLayer, Side};
    use crate::world::propdefs::{def_warnings, PropDefs};
    let kit = kit.trim();
    // A folder is named relative to the backdrop's folder (or the working directory).
    let kit_ref = if kit.starts_with('@') { kit.to_owned() } else {
        let p = std::path::Path::new(kit);
        let p = match &base { Some(b) if p.is_relative() => b.join(p), _ => p.to_path_buf() };
        p.to_string_lossy().into_owned()
    };
    let k = PropDefs::default().kit(std::path::Path::new(&kit_ref))?;
    if k.props.is_empty() { return Err(format!("kit {kit} has no props")); }
    let mut imgs = Vec::new();
    let mut names = Vec::new();
    let mut warnings = Vec::new();
    for (name, def) in k.props.iter().filter(|(n, _)| only.is_none_or(|o| o == n.as_str())) {
        let mut s = backdrop.clone();
        s.props = vec![PropLayer {
            def: format!("{kit_ref}#{name}"), side: Side::Center, lateral: 0.6, spacing: s.motion.loop_length,
            // Hanging things sit high: further out, so they come into view.
            offset: if def.anchor == crate::scene::Anchor::Ceiling { 10.0 } else { 4.5 },
            jitter: 0.0, scale_var: 0.0, density: 1.0, rows: 1, ..PropLayer::default()
        }];
        warnings.extend(def_warnings(&s, base.as_deref()).into_iter().map(|w| format!("{name}: {w}")));
        let img = r.render(&s, 0.0, &RenderOptions { base_dir: base.clone(), ..RenderOptions::default() });
        imgs.push(downscale(&img, scale));
        names.push((name.clone(), def.description.clone()));
    }
    if imgs.is_empty() { return Err(format!("kit {kit} has no prop '{}'", only.unwrap_or(""))); }
    Ok((sheet(&imgs, columns.max(1).min(imgs.len())), names, warnings))
}

/// The scene kits are shown in when no other is given: a road at dusk with nothing beside it, so
/// both a prop's own colours and the light it gives show.
pub fn kit_backdrop() -> Scene {
    let mut s = crate::scene::presets::ALL.iter().find(|p| p.0 == "Night Road").map(|p| (p.1)()).unwrap_or_default();
    s.props.clear();
    s.fixtures.clear();
    s.set_pieces.clear();
    s.particles.clear();
    s
}
