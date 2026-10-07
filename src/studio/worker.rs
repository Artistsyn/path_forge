//! Background threads for the studio: the preview renderer, preset thumbnails, loop checks and exports.
//! The UI thread never renders; it posts the newest request and shows results as they arrive.

use crate::export::{self, ExportJob, ExportReport, Progress};
use crate::review::{self, LoopReport};
use crate::scene::Scene;
use crate::scene::transition::{Branch, ForkChoice, Transition};
use crate::world::{Image, RenderOptions, WorldRenderer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

pub struct Request {
    pub scene: Arc<Scene>,
    pub base_dir: Option<PathBuf>,
    pub distance: f32,
    pub tag: u64,
}

pub struct Frame {
    pub image: Image,
    pub distance: f32,
    pub tag: u64,
    pub ms: f32,
}

/// Renders the newest request only: older ones still queued are dropped.
pub struct Preview {
    tx: Sender<Request>,
    rx: Receiver<Frame>,
}

impl Preview {
    pub fn spawn(ctx: egui::Context) -> Preview {
        let (tx, jobs) = channel::<Request>();
        let (done, rx) = channel::<Frame>();
        std::thread::Builder::new().name("pf-preview".into()).spawn(move || {
            let mut r = WorldRenderer::default();
            while let Ok(mut job) = jobs.recv() {
                while let Ok(newer) = jobs.try_recv() { job = newer; }
                let t0 = Instant::now();
                let opts = RenderOptions { base_dir: job.base_dir.clone(), stats: true, pick: true, ..RenderOptions::default() };
                let image = r.render(&job.scene, job.distance, &opts);
                let ms = t0.elapsed().as_secs_f32() * 1000.0;
                if done.send(Frame { image, distance: job.distance, tag: job.tag, ms }).is_err() { break; }
                ctx.request_repaint();
            }
        }).expect("preview thread");
        Preview { tx, rx }
    }
    pub fn request(&self, r: Request) { let _ = self.tx.send(r); }
    pub fn poll(&self) -> Option<Frame> {
        let mut last = None;
        while let Ok(f) = self.rx.try_recv() { last = Some(f); }
        last
    }
}

/// Frames of every preset across its loop, small, for the gallery: the first frame of each preset
/// first (so the gallery fills quickly), then the rest of each loop. Sends (preset, frame, image).
pub fn thumbnails(ctx: egui::Context, scale: f32, frames: usize) -> Receiver<(usize, usize, Image)> {
    let (tx, rx) = channel();
    std::thread::Builder::new().name("pf-thumbs".into()).spawn(move || {
        let mut r = WorldRenderer::default();
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
        let mut r = WorldRenderer::default();
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
            let mut r = WorldRenderer::default();
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
