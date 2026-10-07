//! Background threads for the studio: the preview renderer, preset thumbnails, loop checks and exports.
//! The UI thread never renders; it posts the newest request and shows results as they arrive.

use crate::export::{self, ExportJob, ExportReport, Progress};
use crate::review::{self, LoopReport};
use crate::scene::Scene;
use crate::scene::transition::{Branch, ForkChoice, Transition};
use crate::world::{Image, RenderOptions, WorldRenderer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// What the studio wants on screen: a scene (by tag) and the frame of its loop at the playhead.
/// While playing, the frames after it are rendered ahead and kept, so playback runs from the cache.
pub struct Want {
    pub scene: Arc<Scene>,
    pub base_dir: Option<PathBuf>,
    pub tag: u64,
    pub index: u32,
    pub ahead: bool,
}

pub struct Frame {
    pub image: Image,
    pub distance: f32,
    pub tag: u64,
    pub index: u32,
    pub ms: f32,
}

/// Most memory the cached frames of a loop may take.
const CACHE_BYTES: usize = 512 << 20;
/// Frames rendered at once: each worker owns a renderer and the rayon pool is shared, so a second
/// frame fills the cores while the first waits at its serial steps.
const WORKERS: usize = 2;

#[derive(Default)]
struct Shared {
    want: Option<Want>,
    /// Finished frames of the wanted tag, by index.
    cache: HashMap<u32, Arc<Frame>>,
    cache_tag: u64,
    busy: HashSet<(u64, u32)>,
    /// The newest frame finished, of any tag: shown while the wanted one is still rendering.
    latest: Option<Arc<Frame>>,
    stop: bool,
}

impl Shared {
    /// Frames in the loop, and how many from the playhead on are worth keeping.
    fn window(&self) -> Option<(u32, u32)> {
        let w = self.want.as_ref()?;
        let n = w.scene.motion.frames();
        let bytes = (w.scene.canvas.width as usize * w.scene.canvas.height as usize * 4).max(1);
        let keep = (CACHE_BYTES / bytes).clamp(1, n as usize) as u32;
        Some((n, if w.ahead { keep } else { 1 }))
    }

    /// The next frame to render: the playhead's, then those after it, skipping any done or under way.
    fn next_job(&self) -> Option<(Arc<Scene>, Option<PathBuf>, u64, u32, f32)> {
        let (n, ahead) = self.window()?;
        let w = self.want.as_ref()?;
        (0..ahead).map(|k| (w.index + k) % n).find(|k| !self.cache.contains_key(k) && !self.busy.contains(&(w.tag, *k))).map(|k| {
            let len = w.scene.motion.loop_length.max(1.0);
            (w.scene.clone(), w.base_dir.clone(), w.tag, k, len * k as f32 / n as f32)
        })
    }

    /// Drop cached frames of another tag, or outside the window ahead of the playhead.
    fn trim(&mut self) {
        let Some(w) = self.want.as_ref() else { return };
        if self.cache_tag != w.tag { self.cache.clear(); self.cache_tag = w.tag; }
        let Some((n, _)) = self.window() else { return };
        let bytes = (w.scene.canvas.width as usize * w.scene.canvas.height as usize * 4).max(1);
        let keep = (CACHE_BYTES / bytes).clamp(1, n as usize) as u32;
        let at = w.index;
        self.cache.retain(|k, _| (k + n - at) % n < keep);
    }
}

/// The preview: renders the wanted frame first and the next ones ahead, on a few threads.
pub struct Preview {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl Preview {
    /// `gpu`: the window's device, when frames should render there (each worker gets buffers of
    /// its own on it, so the two never wait for each other's).
    pub fn spawn(ctx: egui::Context, gpu: Option<Arc<crate::world::gpu::GpuContext>>) -> Preview {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        for k in 0..WORKERS {
            let (sh, ctx, gpu) = (shared.clone(), ctx.clone(), gpu.clone());
            std::thread::Builder::new().name(format!("pf-preview-{k}")).spawn(move || {
                let mut r = WorldRenderer::default();
                if let Some(c) = gpu {
                    match crate::world::gpu::Gpu::new(c) {
                        Ok(g) => r.set_gpu(Some(Arc::new(g))),
                        Err(e) => eprintln!("preview: GPU renderer unavailable, drawing on the CPU: {e}"),
                    }
                }
                let (lock, cv) = &*sh;
                loop {
                    let job = {
                        let mut s = lock.lock().unwrap();
                        loop {
                            if s.stop { return; }
                            if let Some(j) = s.next_job() { s.busy.insert((j.2, j.3)); break j; }
                            s = cv.wait(s).unwrap();
                        }
                    };
                    let (scene, base_dir, tag, index, distance) = job;
                    let t0 = Instant::now();
                    let opts = RenderOptions { base_dir, stats: true, pick: true, ..RenderOptions::default() };
                    let image = r.render(&scene, distance, &opts);
                    let frame = Arc::new(Frame { image, distance, tag, index, ms: t0.elapsed().as_secs_f32() * 1000.0 });
                    let mut s = lock.lock().unwrap();
                    s.busy.remove(&(tag, index));
                    if s.latest.as_ref().map_or(true, |l| l.tag <= tag) { s.latest = Some(frame.clone()); }
                    if s.want.as_ref().is_some_and(|w| w.tag == tag) {
                        s.trim();
                        s.cache.insert(index, frame);
                        s.trim();
                    }
                    drop(s);
                    cv.notify_all();
                    ctx.request_repaint();
                }
            }).expect("preview thread");
        }
        Preview { shared }
    }

    /// Say what is wanted now; cheap to call every UI frame.
    pub fn want(&self, w: Want) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().unwrap();
        let same = s.want.as_ref().is_some_and(|o| o.tag == w.tag && o.index == w.index && o.ahead == w.ahead);
        if same { return; }
        s.want = Some(w);
        s.trim();
        drop(s);
        cv.notify_all();
    }

    /// The wanted frame if it is ready, else the newest one finished (or None before any).
    pub fn frame(&self, tag: u64, index: u32) -> (Option<Arc<Frame>>, bool) {
        let s = self.shared.0.lock().unwrap();
        match s.cache.get(&index).filter(|_| s.cache_tag == tag) {
            Some(f) => (Some(f.clone()), true),
            // A frame rendered ahead of the playhead is not shown early: the last one stays up.
            None => {
                let ahead = |l: &Frame| s.window().is_some_and(|(n, keep)| l.tag == tag && (l.index + n - index) % n < keep);
                (s.latest.clone().filter(|l| !ahead(l)), false)
            }
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().stop = true;
        self.shared.1.notify_all();
    }
}

/// Frames of every preset across its loop, small, for the gallery: the first frame of each preset
/// first (so the gallery fills quickly), then the rest of each loop. Sends (preset, frame, image).
pub fn thumbnails(ctx: egui::Context, scale: f32, frames: usize) -> Receiver<(usize, usize, Image)> {
    let (tx, rx) = channel();
    std::thread::Builder::new().name("pf-thumbs".into()).spawn(move || {
        let mut r = WorldRenderer::auto();
        let scenes: Vec<crate::scene::Scene> = crate::scene::presets::ALL.iter().map(|(_, make)| make()).collect();
        for k in 0..frames.max(1) {
            for (i, s) in scenes.iter().enumerate() {
                let size = ((s.canvas.width as f32 * scale) as u32, (s.canvas.height as f32 * scale) as u32);
                let opts = RenderOptions { size: Some(size), ..RenderOptions::default() };
                let d = s.motion.loop_length * k as f32 / frames.max(1) as f32;
                let img = if s.style.pixel_size > 1 {
                    review::downscale(&r.render(s, d, &RenderOptions::default()), scale)
                } else { r.render(s, d, &opts) };
                if tx.send((i, k, img)).is_err() { return; }
                ctx.request_repaint();
            }
        }
    }).expect("thumbnail thread");
    rx
}

/// Estimate an export's file sizes (and its final colours) in the background.
pub fn estimate(ctx: egui::Context, scene: Arc<Scene>, job: ExportJob) -> Receiver<Result<export::Estimate, String>> {
    let (tx, rx) = channel();
    std::thread::Builder::new().name("pf-estimate".into()).spawn(move || {
        let _ = tx.send(export::estimate(&scene, &job, 6));
        ctx.request_repaint();
    }).expect("estimate thread");
    rx
}

pub fn check_loop(ctx: egui::Context, scene: Scene, base_dir: Option<PathBuf>) -> Receiver<LoopReport> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut r = WorldRenderer::auto();
        let size = ((scene.canvas.width / 2).max(16), (scene.canvas.height / 2).max(16));
        let opts = RenderOptions { size: Some(size), base_dir, ..RenderOptions::default() };
        let _ = tx.send(review::check_loop(&mut r, &scene, &opts, 24));
        ctx.request_repaint();
    });
    rx
}

pub enum ClipKind {
    None,
    Loop,
    Encounter(export::Encounter),
    Transition(Box<Scene>, Option<PathBuf>, Box<Transition>, u32),
    Fork(Box<(Scene, Option<PathBuf>)>, Box<(Scene, Option<PathBuf>)>, Box<ForkChoice>),
}

pub enum ExportEvent {
    Progress(Progress),
    Done(Result<ExportReport, String>),
}

pub struct ExportTask {
    pub rx: Receiver<ExportEvent>,
    pub cancel: Arc<AtomicBool>,
    pub last: Option<Progress>,
}

impl ExportTask {
    pub fn spawn(ctx: egui::Context, scene: Scene, job: ExportJob, kind: ClipKind) -> ExportTask {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::Builder::new().name("pf-export".into()).spawn(move || {
            let c2 = ctx.clone();
            let tx2 = tx.clone();
            let report = move |p| { let _ = tx2.send(ExportEvent::Progress(p)); c2.request_repaint(); };
            let res = match kind {
                ClipKind::Encounter(e) => export::export_encounter(&scene, &job, &e, report, &flag),
                ClipKind::Transition(b, dir, t, entries) => export::export_transition_entries(&scene, &b, dir, &job, &t, entries, report, &flag),
                ClipKind::Fork(l, r, f) => export::export_fork(&scene, (&l.0, l.1.clone()), (&r.0, r.1.clone()), &job, &f, report, &flag),
                _ => export::export(&scene, &job, report, &flag),
            };
            let _ = tx.send(ExportEvent::Done(res));
            ctx.request_repaint();
        }).expect("export thread");
        ExportTask { rx, cancel, last: None }
    }
    pub fn cancel(&self) { self.cancel.store(true, Ordering::Relaxed); }
}

/// What the walk window previews: a transition into one scene, or a fork into two.
#[derive(Clone)]
pub enum WalkSpec {
    Cross { b: Arc<Scene>, b_dir: Option<PathBuf>, t: Transition },
    Fork { l: Arc<Scene>, l_dir: Option<PathBuf>, r: Arc<Scene>, r_dir: Option<PathBuf>, f: ForkChoice, take: Branch },
}

pub struct WalkRequest {
    pub a: Arc<Scene>,
    pub a_dir: Option<PathBuf>,
    pub spec: WalkSpec,
    /// Metres since the walk began.
    pub travel: f32,
    pub size: (u32, u32),
    pub tag: u64,
}

/// A frame of the walk, with the plan behind it.
pub struct WalkFrame {
    pub image: Image,
    pub tag: u64,
    pub length: f32,
    /// What stands at the boundary (or "Fork"), the plan's notes and its warnings.
    pub kind: String,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    /// Where the boundary (or junction) is, metres from the start.
    pub at: f32,
}

/// Renders the newest walk request only.
pub struct WalkPreview {
    tx: Sender<WalkRequest>,
    rx: Receiver<WalkFrame>,
}

impl WalkPreview {
    pub fn spawn(ctx: egui::Context) -> WalkPreview {
        use crate::journey::{CrossWalk, ForkWalk, Place};
        use crate::scene::transition::{plan, plan_fork};
        let (tx, jobs) = channel::<WalkRequest>();
        let (done, rx) = channel::<WalkFrame>();
        std::thread::Builder::new().name("pf-walk".into()).spawn(move || {
            let mut r = WorldRenderer::auto();
            while let Ok(mut job) = jobs.recv() {
                while let Ok(newer) = jobs.try_recv() { job = newer; }
                let opts = RenderOptions { size: Some(job.size), ..RenderOptions::default() };
                let pa = Place { scene: &job.a, dir: job.a_dir.as_deref() };
                let t = job.travel / job.a.motion.speed.max(0.01);
                let frame = match &job.spec {
                    WalkSpec::Cross { b, b_dir, t: tr } => {
                        let c = plan(&job.a, b, tr);
                        let w = CrossWalk::new(c.clone(), 0.0, 0.0);
                        let travel = job.travel.clamp(0.0, w.length());
                        let image = w.frame(&mut r, pa, Place { scene: b, dir: b_dir.as_deref() }, travel, t, t, &opts);
                        WalkFrame { image, tag: job.tag, length: w.length(), kind: c.threshold.name().to_string(), notes: c.notes, warnings: c.warnings, at: c.approach }
                    }
                    WalkSpec::Fork { l, l_dir, r: rs, r_dir, f, take } => {
                        let p = plan_fork(&job.a, l, rs, f);
                        let mut w = ForkWalk::new(p.clone(), 0.0, [0.0, 0.0]);
                        let travel = job.travel.clamp(0.0, w.length());
                        let at = w.decide_by() * 0.5;
                        if travel >= at { w.choose(*take, at); }
                        let image = w.frame(&mut r, pa, Place { scene: l, dir: l_dir.as_deref() }, Place { scene: rs, dir: r_dir.as_deref() }, travel, [t, t, t], &opts);
                        WalkFrame { image, tag: job.tag, length: w.length(), kind: "Fork".into(), notes: p.notes, warnings: p.warnings, at: p.approach }
                    }
                };
                if done.send(frame).is_err() { break; }
                ctx.request_repaint();
            }
        }).expect("walk preview thread");
        WalkPreview { tx, rx }
    }
    pub fn request(&self, r: WalkRequest) { let _ = self.tx.send(r); }
    pub fn poll(&self) -> Option<WalkFrame> {
        let mut last = None;
        while let Ok(f) = self.rx.try_recv() { last = Some(f); }
        last
    }
}
