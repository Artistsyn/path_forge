//! PathForge desktop app. `path_forge [scene.json] [--section path|fixtures/0]` opens the studio; `--v2` opens the old 2.0 editor.

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--v2") {
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_title("PathForge 2.0").with_inner_size([1000.0, 680.0]).with_min_inner_size([780.0, 520.0]),
            ..Default::default()
        };
        return eframe::run_native("PathForge 2.0", native_options, Box::new(|_cc| Ok(Box::new(path_forge::app::PathForgeApp::default()))));
    }
    let section = args.iter().position(|a| a == "--section").and_then(|i| args.get(i + 1)).cloned();
    let open = args.iter().enumerate().find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--section")).map(|(_, a)| std::path::PathBuf::from(a));
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("PathForge").with_inner_size([1320.0, 860.0]).with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("PathForge", native_options, Box::new(move |cc| Ok(Box::new(path_forge::studio::Studio::new(cc, open, section)))))
}
