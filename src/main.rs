// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#![deny(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bubbles;
mod dates;
mod demo;
mod files;
mod list;
mod notify;
mod page;
mod palette;
mod paths;
#[cfg(feature = "screenshot")]
mod screenshot;
mod settings;
mod signin;
mod system;

use eframe::egui::{self, CornerRadius, Stroke, Visuals};
use zirbe_store::Store;

use crate::app::{App, Source};
use crate::palette::Palette;

const APP_ID: &str = "zirbe";
const PRODUCT: &str = "Zirbe";

fn visuals(dark: bool, palette: &Palette) -> Visuals {
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.panel_fill = palette.canvas;
    visuals.window_fill = palette.canvas;
    visuals.extreme_bg_color = palette.field;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.separator);
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);
    visuals
}

fn main() -> eframe::Result {
    files::clear();
    let demo = std::env::args().any(|argument| argument == "--demo");
    let dark = system::dark().unwrap_or(false);
    let palette = if dark { palette::DARK } else { palette::LIGHT };
    let clock = system::clock();

    let viewport = egui::ViewportBuilder::default()
        .with_app_id(APP_ID)
        .with_title(PRODUCT)
        .with_inner_size([1080.0, 760.0])
        .with_min_inner_size([680.0, 440.0]);
    #[cfg(target_os = "macos")]
    let viewport = viewport.with_icon(egui::IconData::default());
    #[cfg(feature = "screenshot")]
    let viewport = match screenshot::Screenshot::size() {
        Some(size) => viewport.with_inner_size(size),
        None => viewport,
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        PRODUCT,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(visuals(dark, &palette));
            let source = if demo {
                Source::Demo(demo::Demo::new())
            } else {
                let ctx = cc.egui_ctx.clone();
                Source::Live(Store::open(
                    paths::config_dir(),
                    paths::cache_dir(),
                    move || ctx.request_repaint(),
                ))
            };
            #[allow(unused_mut)]
            let mut app = App::new(source, palette, clock);
            #[cfg(feature = "screenshot")]
            if let Some(opening) = screenshot::Screenshot::opening() {
                app.open_for_screenshot(&opening);
            }
            Ok(Box::new(app))
        }),
    )
}
