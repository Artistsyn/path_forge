//! Named looks. A style sets how the scene is drawn (style, post and light bands) and leaves the
//! world itself (camera, path, walls, props, lights, motion) alone, so any scene can wear any style.

use super::*;

pub struct StylePreset {
    pub name: &'static str,
    pub about: &'static str,
    pub apply: fn(&mut Scene),
}

fn reset(s: &mut Scene) {
    s.style = Style::default();
    s.light.bands = 0;
    let keep_bloom = s.post.bloom;
    s.post = Post { bloom: keep_bloom, ..Post::default() };
}

fn outline(color: Rgb, objects_only: bool) -> Outline { Outline { enabled: true, color, objects_only } }

pub const ALL: &[StylePreset] = &[
    StylePreset { name: "Clean", about: "Full colour, no filters: the scene as lit.", apply: |s| reset(s) },
    StylePreset {
        name: "Dark Fantasy Pixel",
        about: "Path of Kings feel: chunky 3 px pixels, 32 rich colours, light dither, dark outlines on props, deep vignette.",
        apply: |s| {
            reset(s);
            s.style.pixel_size = 3;
            s.style.palette = Palette::Auto(32);
            s.style.dither = Dither::Bayer4;
            s.style.dither_strength = 0.25;
            s.style.outline = outline([14, 10, 16], true);
            s.style.grade = "Teal Orange".into();
            s.style.grade_strength = 0.3;
            s.post.contrast = 1.12;
            s.post.saturation = 1.1;
            s.post.vignette = 0.5;
        },
    },
    StylePreset {
        name: "16-bit Console",
        about: "SNES-era look: 2 px pixels, 48 colours, fine dither, saturated.",
        apply: |s| {
            reset(s);
            s.style.pixel_size = 2;
            s.style.palette = Palette::Auto(48);
            s.style.dither = Dither::Bayer2;
            s.style.dither_strength = 0.3;
            s.post.saturation = 1.18;
            s.post.contrast = 1.05;
            s.post.vignette = 0.2;
        },
    },
    StylePreset {
        name: "Game Boy",
        about: "Four greens, 3 px pixels, banded light.",
        apply: |s| {
            reset(s);
            s.style.pixel_size = 3;
            s.style.palette = Palette::Named("Game Boy".into());
            s.style.dither = Dither::Bayer4;
            s.style.dither_strength = 0.55;
            s.light.bands = 4;
            // The four greens are a lightness ramp: match by lightness, not hue.
            s.post.saturation = 0.0;
            s.post.contrast = 1.25;
            s.post.vignette = 0.0;
        },
    },
    StylePreset {
        name: "PICO-8",
        about: "The PICO-8 16-colour palette at 4 px pixels with outlined props.",
        apply: |s| {
            reset(s);
            s.style.pixel_size = 4;
            s.style.palette = Palette::Named("PICO-8".into());
            s.style.dither = Dither::Bayer2;
            s.style.dither_strength = 0.4;
            s.style.outline = outline([0, 0, 0], true);
            // Push colours toward the palette's saturated hues instead of its greys.
            s.post.saturation = 1.4;
            s.post.contrast = 1.2;
            s.post.vignette = 0.0;
        },
    },
    StylePreset {
        name: "Painted Storybook",
        about: "Brush strokes (Kuwahara), warm grade and paper texture.",
        apply: |s| {
            reset(s);
            s.style.paint = 4.0;
            s.style.paper = 0.5;
            s.style.grade = "Warm".into();
            s.style.grade_strength = 0.5;
            s.post.saturation = 1.1;
            s.post.vignette = 0.3;
        },
    },
    StylePreset {
        name: "Toon",
        about: "Clean flat shading in three light bands with ink lines on every edge.",
        apply: |s| {
            reset(s);
            s.light.bands = 3;
            s.style.outline = outline([20, 16, 24], false);
            s.post.saturation = 1.15;
            s.post.vignette = 0.15;
        },
    },
    StylePreset {
        name: "Retro CRT",
        about: "2 px pixels, 64 colours, scanlines and a vivid grade.",
        apply: |s| {
            reset(s);
            s.style.pixel_size = 2;
            s.style.palette = Palette::Auto(64);
            s.style.dither = Dither::Bayer2;
            s.style.dither_strength = 0.2;
            s.style.scanlines = 0.6;
            s.style.grade = "Vivid".into();
            s.style.grade_strength = 0.35;
            s.post.vignette = 0.5;
        },
    },
    StylePreset {
        name: "Noir",
        about: "Black and white, hard contrast, film grain.",
        apply: |s| {
            reset(s);
            s.post.saturation = 0.0;
            s.post.contrast = 1.3;
            s.post.grain = 0.25;
            s.post.vignette = 0.6;
        },
    },
];

pub fn find(name: &str) -> Option<&'static StylePreset> {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    ALL.iter().find(|p| key(p.name) == key(name))
}
