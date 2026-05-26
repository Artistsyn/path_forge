
/// Encode a sequence of RGBA frames as a looping animated GIF.
/// `delay_ms` — per-frame delay in milliseconds (e.g. 83 ≈ 12 fps).
pub fn export_gif(
    frames: &[Vec<u8>],
    width:    u16,
    height:   u16,
    delay_ms: u32,
    path:     &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = gif::Encoder::new(file, width, height, &[])?;
    encoder.set_repeat(gif::Repeat::Infinite)?;

    let delay_hundredths = ((delay_ms + 5) / 10) as u16; // round to 10 ms units

    for rgba in frames {
        // Convert RGBA → RGB (GIF has no alpha; discard alpha channel)
        let rgb: Vec<u8> = rgba.chunks(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();

        let mut frame = gif::Frame::from_rgb(width, height, &rgb);
        frame.delay = delay_hundredths;
        encoder.write_frame(&frame)?;
    }

    Ok(())
}

/// Render every frame of one full loop and return them as RGBA vectors.
pub fn render_loop_frames(
    renderer: &mut crate::renderer::PathRenderer,
    settings: &crate::settings::PathForgeSettings,
    fps:      u32,
) -> Vec<Vec<u8>> {
    let loop_tiles  = settings.anim.loop_s as f32;
    let n_frames    = (loop_tiles * fps as f32).round() as u32;
    let n_frames    = n_frames.max(1);
    let frame_step  = loop_tiles / n_frames as f32;
    let cw          = settings.canvas.w();
    let ch          = settings.canvas.h();

    let mut frames = Vec::with_capacity(n_frames as usize);

    for i in 0..n_frames {
        let scroll    = i as f32 * frame_step;
        let global_t  = scroll / loop_tiles; // 0..1

        let mut buf = vec![0u8; cw * ch * 4];
        renderer.render(settings, scroll, global_t, &mut buf);
        frames.push(buf);
    }

    frames
}
