use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use eframe::egui::{self, Color32, Pos2, Rect, Rounding, Sense, Vec2, ViewportCommand};
use log::warn;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::emulator::{self, Display, Emulator, InstructionSettings, Response, Speed};

/// Command line arguments for Jade, the CHIP-8 emulator
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to a Jade settings file (default: jade.toml)
    #[arg(short, long)]
    settings_file: Option<PathBuf>,

    /// ROM file (*.ch8)
    #[arg(value_name = "ROM_FILE")]
    program_file: PathBuf,
}

impl Args {
    pub fn settings_file_path(&self) -> Option<&Path> {
        self.settings_file.as_deref()
    }
}

/// The main application.
pub struct Application {
    emulator: Emulator,
    display: Display,
    key_map: KeyMap,
}

impl Application {
    pub fn new(args: &Args, cc: &eframe::CreationContext<'_>) -> Result<Self, ApplicationError> {
        let settings = load_settings(args.settings_file_path())?;
        let program_data: Vec<u8> = std::fs::read(&args.program_file)?;

        let file_name = args.program_file.file_name().and_then(|s| s.to_str());
        let title = if let Some(file_name) = file_name {
            "Jade".to_string() + " - " + file_name
        } else {
            "Jade".to_string()
        };

        cc.egui_ctx.send_viewport_cmd(ViewportCommand::Title(title));

        let emulator = Emulator::new();
        emulator.load_settings(settings.instructions);
        emulator.load_program(program_data);
        emulator.run_program(Speed::new(settings.instructions_per_second));

        Ok(Application {
            emulator,
            display: Display::default(),
            key_map: KeyMap::from_type(settings.key_map),
        })
    }
}

impl eframe::App for Application {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // We are running in continuous mode, because self.emulator.responses() below does a
        // try_recv(), i.e., we check for messages from the emulator in each frame.
        // (The alternative would be to wake up the GUI thread from the emulator thread, but
        // that seems to be only weakly supported.)
        const FRAME_RATE: f64 = 60.0;
        ctx.request_repaint_after(Duration::from_secs_f64(1.0 / FRAME_RATE));

        // Stop the emulator (thread) when the main window is closed.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.emulator.stop();
        }

        // Send the keys pressed in this frame to the emulator.
        let keys = ctx.input(|i| self.map_keys(&i.keys_down));
        self.emulator.send_keys(&keys);

        // Query for the latest screen display.
        self.emulator.query_display();

        // Get the current content of the display from the responses.
        let responses = self.emulator.responses();
        if let Some(Response::Display(d)) = responses
            .iter()
            .rfind(|&r| matches!(r, Response::Display(_)))
        {
            self.display = d.clone();
        }

        // Log error messages, if there are any.
        for e in responses.iter().filter_map(|response| match response {
            Response::LoadProgram(Err(e)) => Some(e),
            Response::Step(Err(e)) => Some(e),
            Response::RunError(e) => Some(e),
            _ => None,
        }) {
            warn!("emulator error: {}", e);
        }

        // Show the GUI
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    self.show_emulator_screen(ui);
                });
            });
    }
}

impl Application {
    /// Draw the 64x32 CHIP-8 display with blocks of 10x10 pixels.
    fn show_emulator_screen(&self, ui: &mut egui::Ui) {
        const BLOCK_SIZE: f32 = 10.0;

        let screen_dim = Vec2::new(
            emulator::DISPLAY_WIDTH as f32 * BLOCK_SIZE,
            emulator::DISPLAY_HEIGHT as f32 * BLOCK_SIZE,
        );

        let (response, painter) = ui.allocate_painter(screen_dim, Sense::hover());
        let color = Color32::from_gray(128);

        for y in 0..emulator::DISPLAY_HEIGHT {
            for x in 0..emulator::DISPLAY_WIDTH {
                if !self.display.get(x, y) {
                    continue;
                }
                let rect = Rect::from_min_size(
                    Pos2::new(
                        response.rect.left() + x as f32 * BLOCK_SIZE,
                        response.rect.top() + y as f32 * BLOCK_SIZE,
                    ),
                    Vec2::splat(BLOCK_SIZE),
                );
                painter.rect_filled(rect, Rounding::ZERO, color);
            }
        }
    }

    /// Apply the keymap.
    fn map_keys(&self, keys: &HashSet<egui::Key>) -> HashSet<emulator::Key> {
        keys.iter()
            .filter_map(|key| self.key_map.apply(key))
            .collect()
    }
}

#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error(transparent)]
    SettingsFileError(#[from] SettingsFileError),

    #[error("Cannot read program data: {0}")]
    ReadProgramData(#[from] io::Error),
}

#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
enum KeyMapType {
    #[default]
    CommonQWERTY,
    CommonQWERTZ,
    Literal,
}

#[derive(Clone, Debug)]
struct KeyMap {
    map: HashMap<egui::Key, emulator::Key>,
}

impl KeyMap {
    fn from_type(key_map_type: KeyMapType) -> Self {
        let map = match key_map_type {
            KeyMapType::CommonQWERTZ => HashMap::from([
                (egui::Key::Num1, emulator::Key::Num1),
                (egui::Key::Num2, emulator::Key::Num2),
                (egui::Key::Num3, emulator::Key::Num3),
                (egui::Key::Num4, emulator::Key::C),
                (egui::Key::Q, emulator::Key::Num4),
                (egui::Key::W, emulator::Key::Num5),
                (egui::Key::E, emulator::Key::Num6),
                (egui::Key::R, emulator::Key::D),
                (egui::Key::A, emulator::Key::Num7),
                (egui::Key::S, emulator::Key::Num8),
                (egui::Key::D, emulator::Key::Num9),
                (egui::Key::F, emulator::Key::E),
                (egui::Key::Y, emulator::Key::A),
                (egui::Key::X, emulator::Key::Num0),
                (egui::Key::C, emulator::Key::B),
                (egui::Key::V, emulator::Key::F),
            ]),
            KeyMapType::CommonQWERTY => HashMap::from([
                (egui::Key::Num1, emulator::Key::Num1),
                (egui::Key::Num2, emulator::Key::Num2),
                (egui::Key::Num3, emulator::Key::Num3),
                (egui::Key::Num4, emulator::Key::C),
                (egui::Key::Q, emulator::Key::Num4),
                (egui::Key::W, emulator::Key::Num5),
                (egui::Key::E, emulator::Key::Num6),
                (egui::Key::R, emulator::Key::D),
                (egui::Key::A, emulator::Key::Num7),
                (egui::Key::S, emulator::Key::Num8),
                (egui::Key::D, emulator::Key::Num9),
                (egui::Key::F, emulator::Key::E),
                (egui::Key::Z, emulator::Key::A),
                (egui::Key::X, emulator::Key::Num0),
                (egui::Key::C, emulator::Key::B),
                (egui::Key::V, emulator::Key::F),
            ]),
            KeyMapType::Literal => HashMap::from([
                (egui::Key::Num0, emulator::Key::Num0),
                (egui::Key::Num1, emulator::Key::Num1),
                (egui::Key::Num2, emulator::Key::Num2),
                (egui::Key::Num3, emulator::Key::Num3),
                (egui::Key::Num4, emulator::Key::Num4),
                (egui::Key::Num5, emulator::Key::Num5),
                (egui::Key::Num6, emulator::Key::Num6),
                (egui::Key::Num7, emulator::Key::Num7),
                (egui::Key::Num8, emulator::Key::Num8),
                (egui::Key::Num9, emulator::Key::Num9),
                (egui::Key::A, emulator::Key::A),
                (egui::Key::B, emulator::Key::B),
                (egui::Key::C, emulator::Key::C),
                (egui::Key::D, emulator::Key::D),
                (egui::Key::E, emulator::Key::E),
                (egui::Key::F, emulator::Key::F),
            ]),
        };

        KeyMap { map }
    }

    fn apply(&self, key: &egui::Key) -> Option<emulator::Key> {
        self.map.get(key).cloned()
    }
}

#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    key_map: KeyMapType,
    instructions_per_second: usize,
    instructions: InstructionSettings,
}

pub fn load_settings(settings_file: Option<&Path>) -> Result<Settings, SettingsFileError> {
    // Priorities for settings sources
    // 1 If a file path is given on the command line, use that.
    //   If this file does not exist, is not readable etc., complain and exit.
    // 2 If a file "jade.toml" is found in the working directory, use that.
    //   If this file is not readable etc., complain and exit.
    // 3 Use in-built default values

    if let Some(file_path) = settings_file {
        let data = std::fs::read_to_string(file_path)?;
        return Ok(toml::from_str(&data)?);
    }

    const SETTINGS_FILE_NAME: &str = "jade.toml";
    let file_path = Path::new(SETTINGS_FILE_NAME);

    match std::fs::read_to_string(file_path) {
        Ok(data) => Ok(toml::from_str(&data)?),
        Err(e) => match e.kind() {
            io::ErrorKind::NotFound => Ok(Settings::default()),
            _ => Err(SettingsFileError::Read(e)),
        },
    }
}

#[derive(Error, Debug)]
pub enum SettingsFileError {
    #[error("cannot read settings file: {0}")]
    Read(#[from] io::Error),

    #[error("cannot parse settings file: {0}")]
    Parse(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymaps() {
        let qwerty = KeyMap::from_type(KeyMapType::CommonQWERTY);
        let qwertz = KeyMap::from_type(KeyMapType::CommonQWERTZ);
        let literal = KeyMap::from_type(KeyMapType::Literal);

        // "1", top-left on the COSMAC VIP keypad
        assert_eq!(qwerty.apply(&egui::Key::Num1), Some(emulator::Key::Num1));
        assert_eq!(qwertz.apply(&egui::Key::Num1), Some(emulator::Key::Num1));
        assert_eq!(literal.apply(&egui::Key::Num1), Some(emulator::Key::Num1));

        // "F", bottom-right on the COSMAC VIP keypad
        assert_eq!(qwerty.apply(&egui::Key::V), Some(emulator::Key::F));
        assert_eq!(qwertz.apply(&egui::Key::V), Some(emulator::Key::F));
        assert_eq!(literal.apply(&egui::Key::F), Some(emulator::Key::F));

        // QWERTY vs QWERTZ
        assert_eq!(qwerty.apply(&egui::Key::Z), Some(emulator::Key::A));
        assert_eq!(qwertz.apply(&egui::Key::Y), Some(emulator::Key::A));

        // Some unused, out-of-range keys
        assert_eq!(qwerty.apply(&egui::Key::T), None);
        assert_eq!(qwerty.apply(&egui::Key::B), None);
        assert_eq!(literal.apply(&egui::Key::G), None);
    }
}
