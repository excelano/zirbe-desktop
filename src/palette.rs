// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use eframe::egui::Color32;

#[derive(Clone, Copy)]
pub struct Palette {
    pub canvas: Color32,
    pub sidebar: Color32,
    pub field: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub received: Color32,
    pub selected: Color32,
    pub failure: Color32,
}

pub const LIGHT: Palette = Palette {
    canvas: Color32::from_rgb(0xFB, 0xF7, 0xEF),
    sidebar: Color32::from_rgb(0xF2, 0xEE, 0xE5),
    field: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    separator: Color32::from_rgb(0xDD, 0xD8, 0xCD),
    text: Color32::from_rgb(0x1C, 0x1C, 0x1E),
    secondary: Color32::from_rgb(0x6E, 0x6E, 0x73),
    accent: Color32::from_rgb(0x2B, 0x67, 0x4D),
    on_accent: Color32::WHITE,
    received: Color32::from_rgb(0xE9, 0xE9, 0xEB),
    selected: Color32::from_rgb(0xD5, 0xE6, 0xDC),
    failure: Color32::from_rgb(0xC6, 0x28, 0x28),
};

pub const DARK: Palette = Palette {
    canvas: Color32::from_rgb(0x0E, 0x1A, 0x14),
    sidebar: Color32::from_rgb(0x13, 0x22, 0x1B),
    field: Color32::from_rgb(0x1C, 0x2B, 0x24),
    separator: Color32::from_rgb(0x2A, 0x3A, 0x32),
    text: Color32::from_rgb(0xF2, 0xF2, 0xF7),
    secondary: Color32::from_rgb(0x98, 0xA3, 0x9D),
    accent: Color32::from_rgb(0x35, 0x80, 0x5F),
    on_accent: Color32::WHITE,
    received: Color32::from_rgb(0x48, 0x48, 0x4A),
    selected: Color32::from_rgb(0x1F, 0x3D, 0x30),
    failure: Color32::from_rgb(0xFF, 0x8A, 0x80),
};

pub fn monogram(seed: &str) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(0x2B, 0x67, 0x4D),
        Color32::from_rgb(0x8A, 0x5A, 0x2B),
        Color32::from_rgb(0x3D, 0x5A, 0x8A),
        Color32::from_rgb(0x8A, 0x3D, 0x5A),
        Color32::from_rgb(0x5A, 0x4A, 0x8A),
        Color32::from_rgb(0x2B, 0x7A, 0x7A),
        Color32::from_rgb(0x7A, 0x6A, 0x2B),
        Color32::from_rgb(0x6A, 0x6A, 0x6A),
    ];
    let hash = seed.to_lowercase().bytes().fold(5381u32, |hash, byte| {
        hash.wrapping_mul(33) ^ u32::from(byte)
    });
    COLORS[hash as usize % COLORS.len()]
}
