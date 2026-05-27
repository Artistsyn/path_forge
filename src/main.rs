mod settings;
mod tiles;
mod renderer;
mod app;
mod gif_export;
mod node_lab;
mod gpu_effects;
mod gpu_scene;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PathForge 2.0")
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([780.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PathForge 2.0",
        native_options,
        Box::new(|_cc| Ok(Box::new(app::PathForgeApp::default()))),
    )
}
