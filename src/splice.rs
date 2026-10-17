//! Splicing clips into one animation: an intro, a few loops, a transition into the next scene and
//! its loops, as one GIF or WebP to share or to play in something that cannot sequence clips.
//!
//! PathForge's clips are made to join: an intro ends where its loop begins, a transition starts on
//! one loop's frame 0 and ends where the next one begins, an encounter's parts hand over exactly.
//! `splice` only puts them one after another (each repeated as asked) and reports how big a step
//! each join makes next to the clips' own steps, so a join that does not match shows.
//!
//! GIFs joined into a GIF keep their own palettes and pixels exactly (frames are copied, not
//! quantized again); anything else is decoded to colour and encoded once.

use std::path::{Path, PathBuf};

/// One clip of a splice: an animated GIF or WebP, or a folder of PNG frames (`pf export --formats
/// png`), played `times` times.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip { pub path: PathBuf, pub times: u32 }

impl Clip {
    /// `file` or `file*N` (N times).
    pub fn parse(s: &str) -> Result<Clip, String> {
        let (p, n) = match s.rsplit_once('*') {
            Some((p, n)) if n.trim().parse::<u32>().is_ok() => (p, n.trim().parse::<u32>().unwrap()),
            _ => (s, 1),
        };
        if n == 0 { return Err(format!("{s}: a clip must play at least once")); }
        Ok(Clip { path: PathBuf::from(p.trim()), times: n })
    }
}

/// How to write the spliced animation.
#[derive(Clone, Debug)]
pub struct SpliceOptions {
    /// Play forever (true) or once.
    pub looping: bool,
    /// WebP quality 0..100 (100 lossless; default 90).
    pub webp_quality: Option<f32>,
    /// Frames per second for PNG frame folders with no export metadata beside them (default 24).
    pub folder_fps: f32,
}
impl Default for SpliceOptions {
    fn default() -> Self { SpliceOptions { looping: true, webp_quality: None, folder_fps: 24.0 } }
}

/// What a splice wrote.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SpliceReport {
    pub file: PathBuf,
    pub frames: usize,
    pub seconds: f32,
    pub width: u32,
    pub height: u32,
    /// Each join: the clips either side, the mean colour step across it, and the mean step
    /// between frames inside the clips either side (a join much larger than that is a jump).
    pub joins: Vec<Join>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Join { pub from: String, pub to: String, pub step: f32, pub usual: f32 }

/// A clip's frames: GIF frames as they are (to copy into a GIF), or whole RGBA frames.
enum Frames {
    Gif { frames: Vec<gif::Frame<'static>>, global: Option<Vec<u8>>, rgba: Vec<Vec<u8>> },
    Rgba(Vec<Vec<u8>>),
}

/// A clip read in: its size, frames, and the milliseconds each frame shows.
struct Decoded { name: String, w: u32, h: u32, frames: Frames, delays: Vec<u32> }

impl Decoded {
    fn rgba(&self) -> &[Vec<u8>] { match &self.frames { Frames::Gif { rgba, .. } => rgba, Frames::Rgba(f) => f } }
}

/// Join `clips` one after another into `out` (`.gif` or `.webp`).
pub fn splice(clips: &[Clip], out: &Path, o: &SpliceOptions) -> Result<SpliceReport, String> {
    if clips.is_empty() { return Err("nothing to splice: give clips".into()); }
    let ext = out.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext != "gif" && ext != "webp" { return Err(format!("{}: write a .gif or a .webp", out.display())); }
    let decoded: Vec<Decoded> = clips.iter().map(|c| decode(&c.path, o)).collect::<Result<_, _>>()?;
    let (w, h) = (decoded[0].w, decoded[0].h);
    if let Some(d) = decoded.iter().find(|d| (d.w, d.h) != (w, h)) {
        return Err(format!("{} is {}x{} and {} {w}x{h}: clips must be one size to join", d.name, d.w, d.h, decoded[0].name));
    }
    // The order frames play in: (clip, frame) for each repeat.
    let counts: Vec<usize> = decoded.iter().map(|d| d.delays.len()).collect();
    let order: Vec<(usize, usize)> = clips.iter().enumerate().flat_map(|(i, c)| { let n = counts[i]; (0..c.times).flat_map(move |_| (0..n).map(move |k| (i, k))) }).collect();
    let mut warnings = Vec::new();
    let joins = joins(clips, &decoded, &mut warnings);
    let total_ms: u64 = order.iter().map(|&(i, k)| decoded[i].delays[k] as u64).sum();
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) { std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?; }
    if ext == "gif" { write_gif(&decoded, &order, w, h, out, o.looping)?; } else { write_webp(&decoded, &order, w, h, out, o)?; }
    Ok(SpliceReport { file: out.to_path_buf(), frames: order.len(), seconds: total_ms as f32 / 1000.0, width: w, height: h, joins, warnings })
}

fn decode(path: &Path, o: &SpliceOptions) -> Result<Decoded, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    if path.is_dir() { return decode_folder(path, name, o); }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("gif") => decode_gif(&bytes, name),
        Some("webp") => decode_webp(&bytes, name),
        _ => Err(format!("{}: splice reads .gif, .webp and folders of .png frames", path.display())),
    }
}

fn decode_gif(bytes: &[u8], name: String) -> Result<Decoded, String> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut d = opts.read_info(bytes).map_err(|e| format!("{name}: {e}"))?;
    let (w, h) = (d.width() as u32, d.height() as u32);
    let global = d.global_palette().map(|p| p.to_vec());
    let (mut frames, mut rgba, mut delays) = (Vec::new(), Vec::new(), Vec::new());
    // The picture as it builds up, frame over frame (for the joins and for a WebP).
    let mut canvas = vec![0u8; (w * h * 4) as usize];
    while let Some(f) = d.read_next_frame().map_err(|e| format!("{name}: {e}"))? {
        let f = f.clone();
        let pal = f.palette.as_ref().or(global.as_ref()).ok_or_else(|| format!("{name}: a frame with no palette"))?;
        let before = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        for y in 0..f.height as u32 {
            for x in 0..f.width as u32 {
                let (cx, cy) = (f.left as u32 + x, f.top as u32 + y);
                if cx >= w || cy >= h { continue; }
                let idx = f.buffer[(y * f.width as u32 + x) as usize];
                if Some(idx) == f.transparent { continue; }
                let p = ((cy * w + cx) * 4) as usize;
                let c = idx as usize * 3;
                if c + 2 < pal.len() { canvas[p..p + 4].copy_from_slice(&[pal[c], pal[c + 1], pal[c + 2], 255]); }
            }
        }
        rgba.push(canvas.clone());
        delays.push((f.delay as u32 * 10).max(10));
        match f.dispose {
            gif::DisposalMethod::Background => for y in 0..f.height as u32 { for x in 0..f.width as u32 {
                let (cx, cy) = (f.left as u32 + x, f.top as u32 + y);
                if cx < w && cy < h { let p = ((cy * w + cx) * 4) as usize; canvas[p..p + 4].fill(0); }
            }},
            gif::DisposalMethod::Previous => if let Some(b) = before { canvas = b; },
            _ => {}
        }
        frames.push(f);
    }
    if frames.is_empty() { return Err(format!("{name}: no frames")); }
    Ok(Decoded { name, w, h, frames: Frames::Gif { frames, global, rgba }, delays })
}

fn decode_webp(bytes: &[u8], name: String) -> Result<Decoded, String> {
    let d = webp_animation::Decoder::new(bytes).map_err(|e| format!("{name}: {e:?}"))?;
    let (w, h) = d.dimensions();
    let (mut frames, mut delays, mut last) = (Vec::new(), Vec::new(), 0i32);
    for f in d.into_iter() {
        // (The decoder gives each frame's end time.)
        delays.push((f.timestamp() - last).max(10) as u32);
        last = f.timestamp();
        frames.push(f.data().to_vec());
    }
    if frames.is_empty() { return Err(format!("{name}: no frames")); }
    Ok(Decoded { name, w, h, frames: Frames::Rgba(frames), delays })
}

fn decode_folder(dir: &Path, name: String, o: &SpliceOptions) -> Result<Decoded, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).collect();
    files.sort();
    if files.is_empty() { return Err(format!("{}: no .png frames", dir.display())); }
    // The export's own frame rate, from its metadata beside the folder (`<stem>_frames` next to `<stem>.json`).
    let fps = dir.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix("_frames"))
        .and_then(|stem| std::fs::read_to_string(dir.with_file_name(format!("{stem}.json"))).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|m| m["fps"].as_f64()).map_or(o.folder_fps, |f| f as f32).clamp(1.0, 100.0);
    let (mut frames, mut size) = (Vec::new(), (0, 0));
    for f in &files {
        let img = image::open(f).map_err(|e| format!("{}: {e}", f.display()))?.to_rgba8();
        if frames.is_empty() { size = img.dimensions(); } else if img.dimensions() != size { return Err(format!("{}: frames of different sizes", dir.display())); }
        frames.push(img.into_raw());
    }
    let n = frames.len();
    // Whole milliseconds that add up to the clip's length.
    let delays = (0..n).map(|k| ((k + 1) as f32 * 1000.0 / fps).round() as u32 - (k as f32 * 1000.0 / fps).round() as u32).collect();
    Ok(Decoded { name, w: size.0, h: size.1, frames: Frames::Rgba(frames), delays })
}

/// Mean colour step between two frames, 0..255.
fn step(a: &[u8], b: &[u8]) -> f32 {
    let (mut s, mut n) = (0u64, 0u64);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)).step_by(3) {
        s += (0..3).map(|k| p[k].abs_diff(q[k]) as u64).sum::<u64>();
        n += 3;
    }
    if n == 0 { 0.0 } else { s as f32 / n as f32 }
}

/// The step across each join, next to the usual step inside the clips either side.
fn joins(clips: &[Clip], d: &[Decoded], warnings: &mut Vec<String>) -> Vec<Join> {
    let usual = |c: &Decoded| {
        let f = c.rgba();
        let mut s: Vec<f32> = f.windows(2).map(|p| step(&p[0], &p[1])).collect();
        s.sort_by(|a, b| a.total_cmp(b));
        s.get(s.len() / 2).copied().unwrap_or(0.0)
    };
    let mut out = Vec::new();
    let mut add = |a: usize, b: usize, out: &mut Vec<Join>| {
        let (x, y) = (&d[a], &d[b]);
        let s = step(x.rgba().last().unwrap(), &y.rgba()[0]);
        let u = 0.5 * (usual(x) + usual(y));
        if s > 3.0 * u.max(1.0) { warnings.push(format!("{} -> {}: a jump at the join ({s:.1} against {u:.1} between frames); were they exported to follow each other?", x.name, y.name)); }
        out.push(Join { from: x.name.clone(), to: y.name.clone(), step: s, usual: u });
    };
    for (i, c) in clips.iter().enumerate() {
        if c.times > 1 { add(i, i, &mut out); }
        if i + 1 < clips.len() { add(i, i + 1, &mut out); }
    }
    out
}

fn write_gif(d: &[Decoded], order: &[(usize, usize)], w: u32, h: u32, out: &Path, looping: bool) -> Result<(), String> {
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut enc = gif::Encoder::new(std::io::BufWriter::new(file), w as u16, h as u16, &[]).map_err(|e| e.to_string())?;
    enc.set_repeat(if looping { gif::Repeat::Infinite } else { gif::Repeat::Finite(0) }).map_err(|e| e.to_string())?;
    for &(i, k) in order {
        let frame = match &d[i].frames {
            // A GIF's own frame, with its palette carried along.
            Frames::Gif { frames, global, rgba } => {
                let mut f = frames[k].clone();
                if f.palette.is_none() { f.palette = global.clone(); }
                // Its first frame may lean on the picture before it (transparent pixels): give the
                // whole picture instead, so it starts clean after any other clip.
                let whole = f.left == 0 && f.top == 0 && f.width as u32 == w && f.height as u32 == h && f.transparent.is_none();
                if k == 0 && !whole { let mut px = rgba[0].clone(); gif::Frame::from_rgba_speed(w as u16, h as u16, &mut px, 10) } else { f }
            }
            Frames::Rgba(frames) => { let mut px = frames[k].clone(); gif::Frame::from_rgba_speed(w as u16, h as u16, &mut px, 10) }
        };
        let mut frame = frame;
        frame.delay = (d[i].delays[k] / 10).max(1) as u16;
        enc.write_frame(&frame).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn write_webp(d: &[Decoded], order: &[(usize, usize)], w: u32, h: u32, out: &Path, o: &SpliceOptions) -> Result<(), String> {
    use webp_animation::{Encoder, EncoderOptions, EncodingConfig, EncodingType, LossyEncodingConfig};
    let q = o.webp_quality.unwrap_or(90.0).clamp(0.0, 100.0);
    let encoding = if q >= 100.0 { EncodingConfig { encoding_type: EncodingType::Lossless, quality: 100.0, method: 4 } }
        else { EncodingConfig { encoding_type: EncodingType::Lossy(LossyEncodingConfig::default()), quality: q, method: 4 } };
    let opts = EncoderOptions { encoding_config: Some(encoding), kmax: 1, kmin: 0, anim_params: webp_animation::AnimParams { loop_count: if o.looping { 0 } else { 1 } }, ..Default::default() };
    let mut enc = Encoder::new_with_options((w, h), opts).map_err(|e| format!("webp: {e:?}"))?;
    let mut t = 0i32;
    for &(i, k) in order {
        enc.add_frame(&d[i].rgba()[k], t).map_err(|e| format!("webp: {e:?}"))?;
        t += d[i].delays[k] as i32;
    }
    let data = enc.finalize(t).map_err(|e| format!("webp: {e:?}"))?;
    std::fs::write(out, &*data).map_err(|e| format!("{}: {e}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pf_splice_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A clip of `n` frames of a grey that brightens from `from` by `by` a frame.
    fn webp_clip(path: &Path, n: usize, from: u8, by: u8, ms: i32) {
        let mut enc = webp_animation::Encoder::new((8, 6)).unwrap();
        for k in 0..n { let g = from + by * k as u8; enc.add_frame(&[g, g, g, 255].repeat(48), k as i32 * ms).unwrap(); }
        std::fs::write(path, &*enc.finalize(n as i32 * ms).unwrap()).unwrap();
    }

    fn gif_clip(path: &Path, n: usize, from: u8, by: u8) {
        let mut enc = gif::Encoder::new(std::fs::File::create(path).unwrap(), 8, 6, &[]).unwrap();
        for k in 0..n {
            let g = from + by * k as u8;
            let mut px = [g, g, g, 255].repeat(48);
            let mut f = gif::Frame::from_rgba_speed(8, 6, &mut px, 10);
            f.delay = 4;
            enc.write_frame(&f).unwrap();
        }
    }

    #[test]
    fn clips_join_in_order_with_their_repeats_and_timing() {
        let d = temp("order");
        let (intro, lp) = (d.join("intro.webp"), d.join("loop.webp"));
        webp_clip(&intro, 3, 10, 10, 50);
        webp_clip(&lp, 5, 40, 5, 40);
        let out = d.join("all.webp");
        let r = splice(&[Clip::parse(&intro.to_string_lossy()).unwrap(), Clip::parse(&format!("{}*3", lp.display())).unwrap()], &out, &SpliceOptions { webp_quality: Some(100.0), ..Default::default() }).unwrap();
        assert_eq!(r.frames, 3 + 15);
        assert!((r.seconds - (0.15 + 3.0 * 0.2)).abs() < 1e-3, "{}", r.seconds);
        let back = decode(&out, &SpliceOptions::default()).unwrap();
        assert_eq!(back.delays.len(), 18);
        assert_eq!(&back.delays[..4], &[50, 50, 50, 40]);
        let grey: Vec<u8> = back.rgba().iter().map(|f| f[0]).collect();
        assert_eq!(grey, vec![10, 20, 30, 40, 45, 50, 55, 60, 40, 45, 50, 55, 60, 40, 45, 50, 55, 60]);
        // The intro runs into the loop smoothly; the loop's wrap jumps (these test loops do not close).
        assert_eq!(r.joins.len(), 2);
        assert!(r.warnings.iter().any(|w| w.contains("loop.webp -> loop.webp")) && !r.warnings.iter().any(|w| w.contains("intro.webp")));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn gifs_join_into_a_gif_frame_for_frame_and_mixed_clips_too() {
        let d = temp("gif");
        let (a, b) = (d.join("a.gif"), d.join("b.webp"));
        gif_clip(&a, 3, 20, 20);
        webp_clip(&b, 2, 80, 20, 40);
        let out = d.join("all.gif");
        let r = splice(&[Clip::parse(&format!("{}*2", a.display())).unwrap(), Clip::parse(&b.to_string_lossy()).unwrap()], &out, &SpliceOptions { looping: false, ..Default::default() }).unwrap();
        assert_eq!(r.frames, 8);
        let back = decode(&out, &SpliceOptions::default()).unwrap();
        let grey: Vec<u8> = back.rgba().iter().map(|f| f[0]).collect();
        assert_eq!(grey, vec![20, 40, 60, 20, 40, 60, 80, 100]);
        assert_eq!(&back.delays[..3], &[40, 40, 40]);
        // Clips of different sizes do not join.
        let small = d.join("small.webp");
        let mut enc = webp_animation::Encoder::new((4, 4)).unwrap();
        enc.add_frame(&[0u8; 64], 0).unwrap();
        std::fs::write(&small, &*enc.finalize(40).unwrap()).unwrap();
        assert!(splice(&[Clip::parse(&a.to_string_lossy()).unwrap(), Clip::parse(&small.to_string_lossy()).unwrap()], &d.join("x.gif"), &SpliceOptions::default()).unwrap_err().contains("one size"));
        assert!(Clip::parse("x.gif*0").is_err());
        std::fs::remove_dir_all(&d).ok();
    }
}
