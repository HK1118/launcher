#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod models;
mod platform;
mod theme;
mod views;

use app::LauncherApp;
use eframe::egui;
use std::path::Path;
use std::sync::Arc;

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let sys_root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_else(|_| "C:\\Windows".to_string());
    let fonts_dir = Path::new(&sys_root).join("Fonts");

    let font_candidates = [
        fonts_dir.join("meiryo.ttc"),
        fonts_dir.join("YuGothM.ttc"),
        fonts_dir.join("yugothm.ttc"),
        fonts_dir.join("msgothic.ttc"),
    ];

    for font_path in &font_candidates {
        if let Ok(font_data) = std::fs::read(font_path) {
            fonts.font_data.insert(
                "jp_font".to_owned(),
                Arc::new(egui::FontData::from_owned(font_data)),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "jp_font".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, "jp_font".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([580.0, 450.0])
        .with_min_inner_size([450.0, 300.0])
        .with_title("Game Launcher");

    if let Some(icon_data) = icon {
        viewport = viewport.with_icon(icon_data);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Game Launcher",
        options,
        Box::new(|cc| {
            setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(LauncherApp::new(cc)))
        }),
    )
}
