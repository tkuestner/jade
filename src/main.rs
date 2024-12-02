use clap::Parser;
use eframe::egui;

use jade::application::{Application, Args};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args = Args::parse();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 320.0])
            .with_title("Jade the CHIP-8 emulator"),
        ..Default::default()
    };

    Ok(eframe::run_native(
        "Jade",
        options,
        Box::new(|cc| match Application::new(&args, cc) {
            Ok(app) => Ok(Box::new(app)),
            Err(err) => Err(Box::new(err)),
        }),
    )?)
}
