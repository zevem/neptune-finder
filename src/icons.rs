//! Official Tabler Icons 3.49.0, embedded as dedicated native font subsets.
//!
//! Source SVGs, exhaustive mappings, regeneration and MIT notices live in
//! assets/icons/tabler. This is the exact Neptune Terminal glyph mapping and
//! native renderer, shared unchanged for consistent ecosystem controls.

#![allow(dead_code)] // Keep the complete Neptune icon vocabulary available to UI modules.

use eframe::egui::{
    Color32, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Galley, Painter, Pos2,
    Rect, Response, Sense, Shape, StrokeKind, Ui, WidgetInfo, WidgetType,
    emath::{Rot2, TSTransform},
    epaint::TextShape,
    pos2, vec2,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Bell,
    Terminal,
    Plus,
    Close,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Settings,
    Search,
    Sidebar,
    PanelRight,
    SplitVertical,
    SplitHorizontal,
    Folder,
    Grid,
    Keyboard,
    Check,
    ArrowUpRight,
    ArrowUp,
    ArrowDown,
    Minus,
    Maximize,
    Command,
    Sun,
    Moon,
    Copy,
    Code,
    Wrap,
    Ellipsis,
    Refresh,
    Pencil,
    Clipboard,
    Warning,
    Minimize,
    Eraser,
    TextSize,
    Swap,
    Globe,
    Star,
    Image,
    PullRequest,
    Merged,
    Comment,
    Branch,
    Agents,
    File,
    Files,
    FilePlus,
    FolderPlus,
    Trash,
    Paperclip,
    Pause,
    Play,
    Stop,
    Lock,
    /// The pointer that picks an element in a page.
    Pointer,
    Record,
    Person,
    /// The eight reactions a host has, each a small picture.
    ThumbUp,
    ThumbDown,
    Smile,
    Party,
    Unsure,
    Heart,
    Rocket,
    Eyes,
    GitHub,
    ChevronUp,
    ArrowRight,
    ExternalLink,
    ArrowLeft,
    Circle,
    CircleFilled,
    Loader2,
    CircleDashed,
    CircleX,
    CircleCheck,
    Square,
    Checkbox,
    // Finder additions from the same pinned Tabler release.
    Home,
    Download,
    FileText,
    Music,
    Movie,
    Desktop,
    Computer,
    List,
    FileCode,
    FileZip,
    FolderOpen,
    Pin,
    Scissors,
    Info,
    /// A folder listed among files; actions keep the outline `Folder`.
    FolderFilled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Outline,
    Filled,
}

impl Variant {
    fn family(self) -> &'static str {
        match self {
            Self::Outline => "Tabler Outline",
            Self::Filled => "Tabler Filled",
        }
    }

    fn font(self, size: f32) -> FontId {
        FontId::new(size, FontFamily::Name(self.family().into()))
    }
}

mod tabler {
    use super::{Icon, Variant};
    include!("../assets/icons/tabler/glyphs.rs");
}

/// Add dedicated icon families without changing the text families' fallbacks.
pub fn register_fonts(definitions: &mut FontDefinitions) {
    for (variant, bytes) in [
        (
            Variant::Outline,
            include_bytes!("../assets/icons/tabler/outline.ttf").as_slice(),
        ),
        (
            Variant::Filled,
            include_bytes!("../assets/icons/tabler/filled.ttf").as_slice(),
        ),
    ] {
        let family = variant.family();
        definitions
            .font_data
            .insert(family.into(), FontData::from_static(bytes).into());
        definitions
            .families
            .insert(FontFamily::Name(family.into()), vec![family.into()]);
    }
}

/// Paint an icon inside its square design grid. The caller owns the hit area.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    paint_rotated(painter, rect, icon, color, 0.0);
}

/// Paint a glyph rotated clockwise around the design-cell center, in radians.
/// Rotation does not change the caller's layout bounds or hit target.
pub fn paint_rotated(painter: &Painter, rect: Rect, icon: Icon, color: Color32, angle: f32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite()
        || side <= 0.0
        || !angle.is_finite()
        || color.a() == 0
        || !painter.clip_rect().intersects(rect)
    {
        return;
    }
    let (variant, glyph) = tabler::mapping(icon);
    // Fractional/animated bounds reuse whole physical-pixel rasters. Mesh
    // scaling keeps movement smooth without growing the font atlas per frame.
    let pixels_per_point = painter.pixels_per_point();
    let raster_size = (side * pixels_per_point).round().max(1.0) / pixels_per_point;
    let galley = painter.layout_no_wrap(
        glyph.to_string(),
        variant.font(raster_size),
        Color32::PLACEHOLDER,
    );
    let scale = side / raster_size;
    // Preserve Tabler's full 24-point cell and optical offsets, not tight ink
    // centering. Rotate around that same cell for spinners and disclosure.
    let cell = cell_center(&galley, raster_size);
    let rotated = Rot2::from_angle(angle) * cell.to_vec2();
    let shape = if scale == 1.0 {
        Shape::Text(TextShape::new(rect.center() - rotated, galley, color).with_angle(angle))
    } else {
        let mut shape =
            Shape::Text(TextShape::new(Pos2::ZERO - rotated, galley, color).with_angle(angle));
        shape.transform(TSTransform::new(rect.center().to_vec2(), scale));
        shape
    };
    painter.add(shape);
}

/// The centre of a laid-out glyph's design cell, relative to its galley.
///
/// Tabler's em square is the 24-point cell: 0.9 em above the baseline and
/// 0.1 em below it. The font also declares a typographic line gap, which egui
/// adds to the row, so the galley's bounds are taller than the cell and
/// centring them would lift every glyph.
fn cell_center(galley: &Galley, size: f32) -> Pos2 {
    let Some((row, glyph)) = galley
        .rows
        .first()
        .and_then(|row| Some((row, row.glyphs.first()?)))
    else {
        return galley.rect.center();
    };
    let baseline = row.pos + glyph.pos.to_vec2();
    pos2(
        baseline.x + glyph.advance_width * 0.5,
        baseline.y - size * (CELL_ASCENT - 0.5),
    )
}

/// The share of Tabler's em square above the baseline (900 of 1,000 units).
const CELL_ASCENT: f32 = 0.9;

/// A compact, keyboard-focusable icon button with a rounded hover surface.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    button_with_hint(ui, icon, tooltip, "")
}

/// As [`button`], with a keyboard shortcut shown beside the tooltip. The
/// accessible name stays the plain label.
pub fn button_with_hint(ui: &mut Ui, icon: Icon, label: &str, shortcut: &str) -> Response {
    let (_, rect) = ui.allocate_space(vec2(28.0, 28.0));
    let response = ui.interact(rect, ui.id().with(("icon-button", label)), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let surface = rect.shrink(1.0);
        if response.hovered() || response.is_pointer_button_down_on() {
            ui.painter().rect_filled(surface, 7, visuals.weak_bg_fill);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                surface,
                7,
                ui.visuals().selection.stroke,
                StrokeKind::Inside,
            );
        }
        // Pressing settles the glyph slightly, like a physical key.
        let inset = if response.is_pointer_button_down_on() {
            6.5
        } else {
            6.0
        };
        paint(
            ui.painter(),
            rect.shrink(inset),
            icon,
            visuals.fg_stroke.color,
        );
    }
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if shortcut.is_empty() {
        response.on_hover_text(label)
    } else {
        response.on_hover_text(format!("{label}   {shortcut}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    /// The coverage-weighted centre of a painted glyph's ink, in points.
    fn ink_center(pixels_per_point: f32, rect: Rect, icon: Icon, angle: f32) -> Pos2 {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        ctx.set_pixels_per_point(pixels_per_point);
        let mut center = Pos2::ZERO;
        // The second frame paints with the glyph already in the font atlas.
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint_rotated(ui.painter(), rect, icon, Color32::WHITE, angle);
            });
            output.textures_delta.clear();
            let atlas = ctx.fonts(|fonts| fonts.image());
            let [width, height] = atlas.size.map(|size| size as f32);
            let (mut sum, mut weight) = (vec2(0.0, 0.0), 0.0);
            for primitive in ctx.tessellate(output.shapes, pixels_per_point) {
                let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                    continue;
                };
                // Each glyph is a quad: top-left, top-right, bottom-left,
                // bottom-right, possibly rotated.
                for quad in mesh.vertices.chunks_exact(4) {
                    let (u0, v0) = (quad[0].uv.x * width, quad[0].uv.y * height);
                    let (u1, v1) = (quad[3].uv.x * width, quad[3].uv.y * height);
                    let across = quad[1].pos - quad[0].pos;
                    let down = quad[2].pos - quad[0].pos;
                    for v in v0.round() as usize..v1.round() as usize {
                        for u in u0.round() as usize..u1.round() as usize {
                            let alpha = f32::from(atlas[(u, v)].a()) / 255.0;
                            let s = (u as f32 + 0.5 - u0) / (u1 - u0);
                            let t = (v as f32 + 0.5 - v0) / (v1 - v0);
                            sum += (quad[0].pos + across * s + down * t).to_vec2() * alpha;
                            weight += alpha;
                        }
                    }
                }
            }
            center = (sum / weight).to_pos2();
        }
        center
    }

    #[test]
    fn glyphs_stay_centered_in_their_design_cell_at_every_scale_and_angle() {
        // Tabler's plus is symmetric about its cell centre, so its ink centre
        // is the cell centre at any rotation. Text lands on whole physical
        // pixels, leaving under one pixel of error; centring the galley's
        // line box instead lifted glyphs by 1.6 pixels at 150 %.
        for pixels_per_point in [1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
            for origin in [pos2(10.0, 10.0), pos2(10.5, 11.25)] {
                for side in [16.0, 15.5, 14.0, 16.25] {
                    for angle in [
                        0.0,
                        std::f32::consts::FRAC_PI_2,
                        std::f32::consts::TAU / 3.0,
                    ] {
                        let rect = Rect::from_min_size(origin, vec2(side, side));
                        let offset =
                            ink_center(pixels_per_point, rect, Icon::Plus, angle) - rect.center();
                        let limit = 0.75 / pixels_per_point;
                        assert!(
                            offset.x.abs() <= limit && offset.y.abs() <= limit,
                            "glyph off centre by {offset:?} at {pixels_per_point}x, \
                             {angle} rad in {rect:?}"
                        );
                    }
                }
            }
        }
    }
}
