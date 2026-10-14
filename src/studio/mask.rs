//! The mask editor: paint which parts of an image of your own glow (and so give off light), which
//! are solid (take the scene's light, cast a shadow), and which do neither. The mask is saved
//! beside the image as `<name>.mask.png` and goes wherever the image goes.

use super::Studio;
use crate::world::sprites::{mask_path, Mask, MaskPart};
use egui::{Color32, Pos2, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2};
use std::path::PathBuf;

pub(super) struct MaskUi {
    image: PathBuf,
    w: usize,
    h: usize,
    /// The image itself, sRGB with alpha.
    rgba: Vec<u8>,
    mask: Mask,
    /// The mask as it is on disk, to tell whether there is anything to save.
    on_disk: Mask,
    tex: Option<TextureHandle>,
    stale: bool,
    brush: MaskPart,
    radius: f32,
    from: f32,
    last: Option<Pos2>,
}

const GLOW: Color32 = Color32::from_rgb(255, 205, 60);
const CLEAR: Color32 = Color32::from_rgb(70, 140, 255);

impl MaskUi {
    fn open(image: PathBuf) -> Result<MaskUi, String> {
        let img = image::ImageReader::open(&image).map_err(|e| format!("{}: {e}", image.display()))?
            .decode().map_err(|e| format!("{}: {e}", image.display()))?.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w == 0 || h == 0 { return Err(format!("{}: an empty image", image.display())); }
        let mask = Mask::load(&image, w, h);
        Ok(MaskUi { on_disk: mask.clone(), mask, image, w, h, rgba: img.into_raw(), tex: None, stale: true, brush: MaskPart::Glow, radius: (w.max(h) as f32 / 24.0).clamp(1.0, 24.0), from: 0.7, last: None })
    }

    /// The image over a checker, glowing parts washed yellow and shadowless ones blue.
    fn picture(&self) -> egui::ColorImage {
        let mut out = Vec::with_capacity(self.w * self.h);
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let p = &self.rgba[i * 4..i * 4 + 4];
                let a = p[3] as f32 / 255.0;
                let ck = if ((x * 8 / self.w.max(8)) + (y * 8 / self.w.max(8))) % 2 == 0 { 58.0 } else { 46.0 };
                let mut c = [ck + (p[0] as f32 - ck) * a, ck + (p[1] as f32 - ck) * a, ck + (p[2] as f32 - ck) * a];
                let [g, s] = self.mask.px[i];
                let wash = |c: &mut [f32; 3], col: Color32, k: f32| { for (v, t) in c.iter_mut().zip([col.r(), col.g(), col.b()]) { *v += (t as f32 - *v) * k; } };
                if a > 0.0 {
                    if g > 0 { wash(&mut c, GLOW, 0.6 * g as f32 / 255.0); }
                    else if s < 128 { wash(&mut c, CLEAR, 0.5); }
                }
                out.push(Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8));
            }
        }
        egui::ColorImage::new([self.w, self.h], out)
    }
}

impl Studio {
    pub(super) fn open_mask(&mut self, image: PathBuf) {
        match MaskUi::open(image) {
            Ok(m) => self.mask = Some(m),
            Err(e) => self.status = e,
        }
    }

    pub(super) fn mask_window(&mut self, ctx: &egui::Context) {
        // The inspector asks for it by leaving the image's path where this finds it.
        if let Some(p) = ctx.data_mut(|d| d.remove_temp::<String>(egui::Id::new("pf-mask-open"))) { self.open_mask(PathBuf::from(p)); }
        let Some(mut m) = self.mask.take() else { return };
        let (mut open, mut close, mut saved) = (true, false, false);
        let name = m.image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        egui::Window::new(format!("Glow and shadow: {name}")).id(egui::Id::new("mask-editor")).open(&mut open).collapsible(false).resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            ui.label(RichText::new("Paint what each part of the image is. Glowing parts light themselves, give off light in their own colour and cast no shadow; solid parts take the scene's light and cast one.").weak());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Paint");
                // A dab of each part's colour as it shows on the picture, then its name.
                let dab = |ui: &mut egui::Ui, c: Color32| { let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover()); ui.painter().rect_filled(r, 3.0, c); };
                dab(ui, GLOW);
                ui.selectable_value(&mut m.brush, MaskPart::Glow, "Glows").on_hover_text("A flame, a lamp's glass, a lit window, a rune");
                dab(ui, Color32::from_gray(120));
                ui.selectable_value(&mut m.brush, MaskPart::Solid, "Solid").on_hover_text("Takes the scene's light and casts a shadow: what the image is without a mask");
                dab(ui, CLEAR);
                ui.selectable_value(&mut m.brush, MaskPart::Clear, "No shadow").on_hover_text("Neither glows nor casts a shadow: smoke, glass, thin chains");
                ui.separator();
                ui.label("Brush");
                ui.add(egui::Slider::new(&mut m.radius, 0.5..=(m.w.max(m.h) as f32 / 4.0).max(2.0)).suffix(" px").max_decimals(1));
            });
            ui.horizontal(|ui| {
                if ui.button("Mark the light parts").on_hover_text("Everything at least this light glows; the rest is solid").clicked() {
                    m.mask = Mask::from_light(&m.rgba, m.w, m.h, m.from);
                    m.stale = true;
                }
                ui.add(egui::Slider::new(&mut m.from, 0.2..=1.0).text("from this light").max_decimals(2));
                if ui.button("All solid").on_hover_text("Start over: nothing glows, everything casts a shadow").clicked() { m.mask = Mask::solid(m.w, m.h); m.stale = true; }
            });
            ui.add_space(4.0);
            // The picture, as large as fits, in whole steps for small pixel art.
            let room = Vec2::new(620.0, (ctx.screen_rect().height() - 330.0).clamp(160.0, 620.0));
            let mut scale = (room.x / m.w as f32).min(room.y / m.h as f32);
            if scale > 1.0 { scale = scale.floor(); }
            let size = Vec2::new(m.w as f32 * scale, m.h as f32 * scale);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
            if m.stale {
                let ci = m.picture();
                let opt = if scale >= 2.0 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
                match &mut m.tex { Some(t) => t.set(ci, opt), None => m.tex = Some(ctx.load_texture("mask-editor", ci, opt)) }
                m.stale = false;
            }
            if let Some(t) = &m.tex { ui.painter().image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); }
            // Paint along the pointer's way, so a quick stroke leaves no gaps.
            let down = resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on();
            if let (true, Some(p)) = (down, resp.interact_pointer_pos()) {
                let to_px = |p: Pos2| ((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale);
                let from = m.last.unwrap_or(p);
                let steps = ((p - from).length() / (m.radius * scale * 0.5).max(1.0)).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let q = from + (p - from) * (k as f32 / steps as f32);
                    let (x, y) = to_px(q);
                    m.mask.paint(x, y, m.radius, m.brush);
                }
                m.last = Some(p);
                m.stale = true;
            } else { m.last = None; }
            if let Some(p) = resp.hover_pos() {
                ui.painter().circle_stroke(p, m.radius * scale, egui::Stroke::new(1.0, Color32::WHITE));
            }
            ui.add_space(4.0);
            let alpha: Vec<u8> = m.rgba.chunks_exact(4).map(|p| p[3]).collect();
            let share = m.mask.glowing(&alpha);
            ui.label(RichText::new(if share > 0.0 {
                format!("{:.0}% of it glows. As a light, it shines from the middle of the glowing part in that part's colour; as a prop, it lights the ground round it.", share * 100.0)
            } else { "Nothing glows yet: it is lit by the scene like any other prop.".to_owned() }).weak());
            ui.horizontal(|ui| {
                let dirty = m.mask != m.on_disk;
                if ui.add_enabled(dirty, egui::Button::new(RichText::new("Save mask").strong())).clicked() {
                    // A mask that says nothing is no mask: leave no file behind.
                    let r = if m.mask.is_plain() { let f = mask_path(&m.image); if f.is_file() { std::fs::remove_file(&f).map_err(|e| e.to_string()) } else { Ok(()) } }
                        else { m.mask.save(&m.image).map(|_| ()) };
                    match r {
                        Ok(()) => { m.on_disk = m.mask.clone(); saved = true; self.status = format!("Saved the mask for {name}"); }
                        Err(e) => self.status = e,
                    }
                }
                if ui.add_enabled(dirty, egui::Button::new("Revert")).on_hover_text("Back to the saved mask").clicked() { m.mask = m.on_disk.clone(); m.stale = true; }
                if ui.button("Close").clicked() { close = true; }
                if dirty { ui.label(RichText::new("not saved").weak().italics()); }
            });
        });
        // The preview reads the mask with the image: draw the frame again.
        if saved { self.tag += 1; }
        if open && !close { self.mask = Some(m); }
    }
}
