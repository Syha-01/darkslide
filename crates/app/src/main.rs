mod app;
mod cli;
mod dialogs;
mod doc;
mod edit_panel;
mod filmstrip;
mod presets_panel;
#[cfg(feature = "screenshot")]
mod screenshot;
mod theme;
mod tool_panels;
mod viewer;
mod widgets;
mod worker;

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--cli").unwrap_or(false) {
        return cli::run(args[1..].to_vec());
    }
    #[cfg(feature = "screenshot")]
    if args.first().map(|a| a == "--screenshot").unwrap_or(false) {
        return screenshot::run(args);
    }
    #[cfg(feature = "screenshot")]
    if args.first().map(|a| a == "--smoke").unwrap_or(false) {
        return screenshot::smoke(args);
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: darkslide [FILE|FOLDER]...\n       darkslide --cli --help   (batch mode)");
        return Ok(());
    }
    let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Darkslide")
            .with_app_id("darkslide")
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Darkslide", opts, Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths))))).map_err(|e| anyhow::anyhow!("{e}"))
}
