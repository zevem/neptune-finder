//! Neptune's bundled Geist interface fonts and JetBrains Mono for paths/code.
//!
//! The font bytes, family names, weights and fallback order are copied from
//! Neptune Terminal. Installing them needs no startup filesystem access.

use eframe::egui::{self, FontFamily};

pub fn install(ctx: &egui::Context) {
    let mut definitions = egui::FontDefinitions::default();
    crate::icons::register_fonts(&mut definitions);
    for (name, bytes) in [
        (
            "Geist",
            include_bytes!("../assets/fonts/Geist-Regular.ttf").as_slice(),
        ),
        (
            "GeistMedium",
            include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice(),
        ),
        (
            "GeistSemiBold",
            include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice(),
        ),
        (
            "JetBrains",
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
        ),
        (
            "JetBrainsBold",
            include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf").as_slice(),
        ),
    ] {
        definitions
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    definitions
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains".into());
    let proportional = definitions
        .families
        .entry(FontFamily::Proportional)
        .or_default();
    proportional.insert(0, "Geist".into());
    let proportional = proportional.clone();
    for (family, face) in [
        (crate::theme::MEDIUM, "GeistMedium"),
        (crate::theme::SEMIBOLD, "GeistSemiBold"),
    ] {
        let mut fonts = proportional.clone();
        fonts[0] = face.into();
        definitions
            .families
            .insert(FontFamily::Name(family.into()), fonts);
    }
    ctx.set_fonts(definitions);
}
