//! Klip's look: graphite surfaces, a soft "paper" blue accent, Inter type,
//! and tactile controls — gradient buttons with a top highlight and a soft
//! drop shadow, sliding segmented controls, focus rings on inputs.

use egui::epaint::Shadow;
use egui::{
    Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::path::PathBuf;
use std::sync::Arc;

// Surfaces (darkest → lightest).
pub const BG0: Color32 = Color32::from_rgb(0x0f, 0x10, 0x13);
pub const BG1: Color32 = Color32::from_rgb(0x17, 0x18, 0x1c);
pub const BG2: Color32 = Color32::from_rgb(0x1f, 0x21, 0x26);
pub const BG3: Color32 = Color32::from_rgb(0x29, 0x2b, 0x31);
pub const BG4: Color32 = Color32::from_rgb(0x35, 0x38, 0x3f);
pub const BORDER: Color32 = Color32::from_rgb(0x2c, 0x2e, 0x34);
pub const BORDER_HI: Color32 = Color32::from_rgb(0x40, 0x43, 0x4b);

// Text.
pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xf0, 0xf3);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9b, 0xa0, 0xaa);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x63, 0x68, 0x72);

// Accent: the soft "paper" blue.
pub const ACCENT: Color32 = Color32::from_rgb(0x78, 0xa4, 0xe8);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0x96, 0xba, 0xf2);
/// Text/icons drawn on top of the accent colour.
pub const ON_ACCENT: Color32 = Color32::WHITE;
/// Snap / alignment guides.
pub const GUIDE: Color32 = Color32::from_rgb(0xf5, 0xc2, 0x4c);

pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const VIDEO_CLIP: Color32 = Color32::from_rgb(0x26, 0x2b, 0x33);
pub const TEXT_CLIP: Color32 = Color32::from_rgb(0x96, 0x83, 0xe6);
pub const CAPTION_CLIP: Color32 = Color32::from_rgb(0xd6, 0x9e, 0x3c);
pub const AUDIO_CLIP: Color32 = Color32::from_rgb(0x2e, 0x8c, 0x66);
pub const DANGER: Color32 = Color32::from_rgb(0xef, 0x6b, 0x6b);
pub const SUCCESS: Color32 = Color32::from_rgb(0x5c, 0xc8, 0x8f);

/// Controls (buttons, inputs, segmented controls).
pub const R_SM: u8 = 8;
/// Cards and tiles.
pub const R_MD: u8 = 10;
/// Hover / selection fade duration.
pub const ANIM: f32 = 0.16;

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const INTER_DISPLAY_BOLD: &[u8] = include_bytes!("../assets/fonts/InterDisplay-Bold.ttf");

/// Font used for text burned into the video (preview + export).
pub fn text_family() -> FontFamily {
    FontFamily::Name("klip_text".into())
}
/// Display headings and the wordmark.
pub fn heading_family() -> FontFamily {
    FontFamily::Name("ui_display".into())
}
pub fn medium_family() -> FontFamily {
    FontFamily::Name("ui_medium".into())
}
pub fn semibold_family() -> FontFamily {
    FontFamily::Name("ui_semibold".into())
}
/// Phosphor Bold — same codepoints as the regular icons, heavier strokes.
pub fn bold_icons() -> FontFamily {
    FontFamily::Name("icons_bold".into())
}

/// Locate a font file via fontconfig.
fn fc_match(pattern: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
        .ok()?;
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    p.is_file().then_some(p)
}

/// The bold display font used for on-video text, shared by preview and export.
pub fn find_text_font() -> Option<PathBuf> {
    for pat in ["Montserrat:style=Bold", "Poppins:style=Bold", "Inter:style=Bold", "sans-serif:bold"] {
        if let Some(p) = fc_match(pat) {
            let name = p.to_string_lossy().to_lowercase();
            // fontconfig always returns *something*; only accept real matches for named families.
            let fam = pat.split(':').next().unwrap().to_lowercase();
            if fam == "sans-serif" || name.contains(&fam) {
                return Some(p);
            }
        }
    }
    None
}

pub fn install(ctx: &egui::Context, text_font: Option<&PathBuf>) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "inter", INTER_REGULAR);
    add(&mut fonts, "inter_medium", INTER_MEDIUM);
    add(&mut fonts, "inter_semibold", INTER_SEMIBOLD);
    add(&mut fonts, "inter_display", INTER_DISPLAY_BOLD);
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Icons right after Inter. (The bundled Inter has its Private Use Area
    // glyphs stripped so they can't shadow Phosphor's icon codepoints.)
    let prop = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    prop.retain(|f| f != "phosphor");
    prop.insert(1, "phosphor".into());
    // Heavier icon weight for buttons, where the regular weight looks thin.
    egui_phosphor::add_font_bytes_as_family(&mut fonts, "icons_bold", egui_phosphor::Variant::Bold.font_bytes());

    // System fallbacks for scripts Inter lacks (Sinhala, CJK, full-width
    // punctuation…), found via fontconfig by character coverage.
    let mut seen = std::collections::HashSet::new();
    for (k, charset) in ["0dc3", "ff1a"].iter().enumerate() {
        let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{file}|%{index}", &format!("sans:charset={charset}")]).output() else {
            continue;
        };
        let spec = String::from_utf8_lossy(&out.stdout).to_string();
        let (file, index) = spec.split_once('|').unwrap_or((spec.as_str(), "0"));
        if file.is_empty() || !seen.insert(file.to_string()) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(file) {
            let mut fd = FontData::from_owned(bytes);
            // fontconfig packs the variable-font instance into the high bits.
            fd.index = index.trim().parse::<u32>().unwrap_or(0) & 0xFFFF;
            let name = format!("fallback{k}");
            fonts.font_data.insert(name.clone(), Arc::new(fd));
            fonts.families.entry(FontFamily::Proportional).or_default().push(name);
        }
    }

    // Every named family falls back to the regular stack (incl. icons).
    let fallback: Vec<String> = fonts.families[&FontFamily::Proportional].clone();
    let with = |first: &str| std::iter::once(first.to_string()).chain(fallback.iter().cloned()).collect::<Vec<_>>();
    fonts.families.insert(medium_family(), with("inter_medium"));
    fonts.families.insert(semibold_family(), with("inter_semibold"));
    fonts.families.insert(heading_family(), with("inter_display"));
    let mut text_fam = vec![];
    if let Some(bytes) = text_font.and_then(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("text".into(), Arc::new(FontData::from_owned(bytes)));
        text_fam.push("text".to_string());
    }
    text_fam.push("inter_semibold".into());
    text_fam.extend(fallback.iter().cloned());
    fonts.families.insert(text_family(), text_fam);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG1;
    v.window_fill = BG2;
    v.extreme_bg_color = BG0;
    v.faint_bg_color = BG2;
    v.code_bg_color = BG2;
    v.window_stroke = Stroke::new(1.0, BORDER_HI);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.window_shadow = Shadow { offset: [0, 18], blur: 50, spread: 0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = Shadow { offset: [0, 8], blur: 22, spread: 0, color: Color32::from_black_alpha(130) };
    v.selection.bg_fill = ACCENT.gamma_multiply(0.38);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT_HI;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);

    let r = CornerRadius::same(R_SM);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = BG2;
    w.noninteractive.weak_bg_fill = BG2;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = BG3;
    w.inactive.weak_bg_fill = BG3;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER_HI);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = BG4;
    w.hovered.weak_bg_fill = BG4;
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x52, 0x56, 0x60));
    w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = BG2;
    w.active.weak_bg_fill = BG2;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(14.0, 7.0);
        s.spacing.interact_size = Vec2::new(40.0, 32.0);
        s.spacing.slider_width = 150.0;
        s.spacing.slider_rail_height = 4.0;
        s.spacing.window_margin = Margin::same(20);
        s.spacing.menu_margin = Margin::same(8);
        s.spacing.scroll.bar_width = 6.0;
        s.spacing.scroll.floating = true;
        s.animation_time = ANIM;
        use egui::TextStyle as TS;
        s.text_styles.insert(TS::Body, FontId::proportional(14.0));
        s.text_styles.insert(TS::Button, FontId::new(14.0, medium_family()));
        s.text_styles.insert(TS::Small, FontId::proportional(12.0));
        s.text_styles.insert(TS::Heading, FontId::new(18.0, heading_family()));
        s.text_styles.insert(TS::Monospace, FontId::monospace(13.0));
    });
}

// ----------------------------------------------------------------- animation

/// 0→1 eased value that follows `on` over [`ANIM`] seconds.
pub fn anim(ui: &egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, on: bool) -> f32 {
    ui.ctx().animate_bool_with_time(ui.id().with(("anim", id)), on, ANIM)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// Opacity for content that should fade in whenever `key` changes.
pub fn fade_in(ui: &egui::Ui, salt: &str, key: u64) -> f32 {
    let id = egui::Id::new(("fade", salt));
    let now = ui.input(|i| i.time);
    let (prev, since) = ui.ctx().data(|d| d.get_temp::<(u64, f64)>(id)).unwrap_or((key, now - 1.0));
    let since = if prev != key { now } else { since };
    ui.ctx().data_mut(|d| d.insert_temp(id, (key, since)));
    let t = ((now - since) / 0.18) as f32;
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

// ------------------------------------------------------------------ drawing

/// Dashed outline that follows rounded corners.
pub fn dashed_rrect(p: &egui::Painter, r: Rect, rad: f32, stroke: Stroke) {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    let mut pts = vec![];
    let corners = [
        (Pos2::new(r.right() - rad, r.top() + rad), -90.0f32),
        (Pos2::new(r.right() - rad, r.bottom() - rad), 0.0),
        (Pos2::new(r.left() + rad, r.bottom() - rad), 90.0),
        (Pos2::new(r.left() + rad, r.top() + rad), 180.0),
    ];
    for (c, start) in corners {
        for i in 0..=8 {
            let a = (start + 90.0 * i as f32 / 8.0).to_radians();
            pts.push(c + Vec2::new(a.cos(), a.sin()) * rad);
        }
    }
    pts.push(pts[0]);
    p.extend(Shape::dashed_line(&pts, stroke, 6.0, 5.0));
}

/// Flat surface with an optional hairline border.
fn surface(p: &egui::Painter, r: Rect, rad: u8, fill: Color32, border: Color32) {
    p.rect_filled(r, CornerRadius::same(rad), fill);
    if border.a() > 0 {
        p.rect_stroke(r, CornerRadius::same(rad), Stroke::new(1.0, border), StrokeKind::Inside);
    }
}

// ------------------------------------------------------------------ widgets

/// Section label used in side panels.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).color(TEXT_FAINT).family(semibold_family()));
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    #[allow(dead_code)]
    Danger,
}

/// A clean, flat button that eases lighter on hover and a touch darker when
/// pressed. `text` may start with a Phosphor icon (`"{icon}  Label"`).
pub fn rich_button(ui: &mut egui::Ui, text: impl Into<String>, kind: Kind, min_w: f32) -> Response {
    let text = text.into();
    // Optional leading icon ("{icon}  Label"), drawn with the bold icon weight.
    let (icon, label) = match text.char_indices().next() {
        Some((_, c)) if ('\u{E000}'..='\u{F8FF}').contains(&c) => {
            let rest = text[c.len_utf8()..].trim_start().to_string();
            (Some(c.to_string()), rest)
        }
        _ => (None, text),
    };
    let font = FontId::new(14.0, medium_family());
    let galley = ui.painter().layout_no_wrap(label, font, Color32::WHITE);
    let icon_g = icon.map(|i| ui.painter().layout_no_wrap(i, FontId::new(15.0, bold_icons()), Color32::WHITE));
    let gap = 7.0;
    let content_w = galley.size().x + icon_g.as_ref().map_or(0.0, |g| g.size().x + gap);
    let size = Vec2::new((content_w + 30.0).max(min_w), 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let enabled = ui.is_enabled();
    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), enabled && resp.hovered(), ANIM);
    let pr = ui.ctx().animate_bool_with_time(resp.id.with("p"), enabled && resp.is_pointer_button_down_on(), 0.06);
    let (fill, border, fg) = match kind {
        Kind::Primary => (ACCENT, Color32::TRANSPARENT, ON_ACCENT),
        Kind::Secondary => (BG3, BORDER_HI, TEXT),
        Kind::Danger => (DANGER, Color32::TRANSPARENT, Color32::WHITE),
    };
    let fill = mix(mix(fill, Color32::WHITE, 0.07 * h), Color32::BLACK, 0.1 * pr);
    let r = rect;
    let p = ui.painter();
    let fade = if enabled { 1.0 } else { 0.45 };
    surface(p, r, R_SM, fill.gamma_multiply(fade), border.gamma_multiply(fade));
    let fg = fg.gamma_multiply(if enabled { 1.0 } else { 0.5 });
    let mut x = r.center().x - content_w / 2.0;
    if let Some(g) = icon_g {
        let w = g.size().x;
        p.galley(Pos2::new(x, r.center().y - g.size().y / 2.0), g, fg);
        x += w + gap;
    }
    p.galley(Pos2::new(x, r.center().y - galley.size().y / 2.0), galley, fg);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// The main action on screen.
pub fn primary_button(ui: &mut egui::Ui, text: impl Into<String>) -> Response {
    rich_button(ui, text, Kind::Primary, 0.0)
}

/// Neutral raised button for secondary actions.
pub fn secondary_button(ui: &mut egui::Ui, text: impl Into<String>) -> Response {
    rich_button(ui, text, Kind::Secondary, 0.0)
}

/// Quiet icon button for toolbars: no chrome until hovered.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), if enabled { Sense::click() } else { Sense::hover() });
    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), enabled && resp.hovered(), ANIM);
    let pr = ui.ctx().animate_bool_with_time(resp.id.with("p"), enabled && resp.is_pointer_button_down_on(), 0.06);
    let p = ui.painter();
    if h > 0.01 {
        p.rect_filled(rect.shrink(1.0), CornerRadius::same(R_SM), mix(BG3, BG2, pr).gamma_multiply(h));
    }
    let col = if enabled { mix(TEXT_DIM, TEXT, h) } else { TEXT_FAINT.gamma_multiply(0.7) };
    p.text(rect.center() + Vec2::new(0.0, 0.5 * pr), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), col);
    let resp = resp.on_hover_text(tip);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// Text link-style button (accent text, no chrome).
pub fn link_button(ui: &mut egui::Ui, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::new(13.5, medium_family()), ACCENT);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(12.0, 12.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), ANIM);
    let p = ui.painter();
    if h > 0.01 {
        p.rect_filled(rect, CornerRadius::same(R_SM), ACCENT.gamma_multiply(0.12 * h));
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, mix(ACCENT, ACCENT_HI, h));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Apple-style segmented control with a sliding raised knob.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let font = FontId::new(13.0, medium_family());
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT))
        .collect();
    let seg_w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 28.0;
    let pad = 3.0;
    let size = Vec2::new(seg_w * options.len() as f32 + pad * 2.0, 34.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let id = ui.id().with(("segmented", rect.min.x as i32, rect.min.y as i32));
    let sel = options.iter().position(|(v, _)| *v == *value).unwrap_or(0);
    let pos = ui.ctx().animate_value_with_time(id, sel as f32, 0.2);

    let p = ui.painter();
    // Inset track.
    surface(p, rect, R_SM + 2, BG0, BORDER);
    // Knob.
    let knob = Rect::from_min_size(
        Pos2::new(rect.left() + pad + pos * seg_w, rect.top() + pad),
        Vec2::new(seg_w, size.y - pad * 2.0),
    );
    surface(p, knob, R_SM, BG4, Color32::TRANSPARENT);

    let mut changed = false;
    for (i, ((v, _), g)) in options.iter().zip(galleys).enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.left() + pad + i as f32 * seg_w, rect.top() + pad), knob.size());
        let resp = ui.interact(r, id.with(i), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let t = (1.0 - (pos - i as f32).abs()).clamp(0.0, 1.0);
        let h = ui.ctx().animate_bool_with_time(id.with(("h", i)), resp.hovered(), ANIM);
        let col = mix(mix(TEXT_DIM, TEXT, h * 0.6), Color32::WHITE, t);
        ui.painter().galley(r.center() - g.size() / 2.0, g, col);
        if resp.clicked() && *value != *v {
            *value = *v;
            changed = true;
        }
    }
    changed
}

/// Single-line text field. Focus shows as a clean accent border (no glow)
/// that eases in.
pub fn text_input(ui: &mut egui::Ui, text: &mut String, width: f32) -> Response {
    input_field(ui, text, width, Margin::symmetric(13, 9), false)
}

/// Borderless until hovered/focused — for inline renaming (e.g. project name).
pub fn inline_input(ui: &mut egui::Ui, text: &mut String, width: f32) -> Response {
    input_field(ui, text, width, Margin::symmetric(10, 6), true)
}

fn input_field(ui: &mut egui::Ui, text: &mut String, width: f32, margin: Margin, subtle: bool) -> Response {
    let bg = ui.painter().add(Shape::Noop);
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .frame(egui::Frame::NONE.inner_margin(margin))
            .desired_width(width - margin.sum().x)
            .text_color(TEXT),
    );
    let ease = egui::emath::easing::cubic_out;
    let f = ui.ctx().animate_bool_with_time_and_easing(resp.id.with("focus"), resp.has_focus(), 0.22, ease);
    let h = ui.ctx().animate_bool_with_time_and_easing(resp.id.with("hover"), resp.hovered(), 0.22, ease);
    let r = resp.rect;
    let (fill, border) = if subtle {
        (
            mix(mix(Color32::TRANSPARENT, BG2, h), BG0, f),
            mix(mix(Color32::TRANSPARENT, BORDER, h), ACCENT.gamma_multiply(0.7), f),
        )
    } else {
        (BG0, mix(mix(BORDER, BORDER_HI, h), ACCENT.gamma_multiply(0.7), f))
    };
    let mut shapes = vec![Shape::rect_filled(r, CornerRadius::same(R_SM), fill)];
    if border.a() > 0 {
        shapes.push(Shape::rect_stroke(r, CornerRadius::same(R_SM), Stroke::new(1.0, border), StrokeKind::Inside));
    }
    ui.painter().set(bg, Shape::Vec(shapes));
    resp
}

// ------------------------------------------------------------- sliders

/// How a number is shown and stepped in a [`value_box`].
#[derive(Clone, Copy)]
pub struct Num {
    pub suffix: &'static str,
    pub decimals: usize,
    pub step: f32,
}

pub const fn num(suffix: &'static str, decimals: usize, step: f32) -> Num {
    Num { suffix, decimals, step }
}

/// Slim slider: thin rail, light fill, small white pill knob.
pub fn slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(width.max(40.0), 24.0), Sense::click_and_drag());
    let pad = 5.0;
    let rail = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - pad * 2.0, 3.0));
    if resp.is_pointer_button_down_on() || resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
            let nv = lo + t * (hi - lo);
            if (nv - *v).abs() > f32::EPSILON {
                *v = nv;
                resp.mark_changed();
            }
        }
    }
    let t = if hi > lo { ((*v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let x = rail.left() + t * rail.width();
    let active = resp.hovered() || resp.dragged();
    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), active, ANIM);
    let p = ui.painter();
    p.rect_filled(rail, CornerRadius::same(2), BG4);
    p.rect_filled(Rect::from_min_max(rail.min, Pos2::new(x, rail.max.y)), CornerRadius::same(2), mix(Color32::from_gray(200), Color32::WHITE, h));
    let knob = Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::new(6.0 + h, 14.0 + 2.0 * h));
    p.add(Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_black_alpha(120) }.as_shape(knob, CornerRadius::same(3)));
    p.rect_filled(knob, CornerRadius::same(3), Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Editable number box with ▲▼ steppers: click to type, drag to scrub.
pub fn value_box(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, n: Num) -> bool {
    let mut changed = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = 30.0;
        let w = &mut ui.visuals_mut().widgets;
        for st in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            st.weak_bg_fill = BG0;
            st.bg_fill = BG0;
            st.corner_radius = CornerRadius::same(R_SM);
            st.expansion = 0.0;
        }
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        w.hovered.bg_stroke = Stroke::new(1.0, BORDER_HI);
        w.active.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
        let dv = egui::DragValue::new(v)
            .range(range.clone())
            .speed(n.step.max(0.01) * 0.5)
            .suffix(n.suffix)
            .min_decimals(n.decimals)
            .max_decimals(n.decimals);
        let dv = ui.add_sized(Vec2::new(64.0, 30.0), dv);
        changed |= dv.changed();
        let base = dv.id;

        // Steppers.
        let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 30.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(R_SM), BG3);
        ui.painter().rect_stroke(r, CornerRadius::same(R_SM), Stroke::new(1.0, BORDER), StrokeKind::Inside);
        for k in 0..2 {
            let half = Rect::from_min_size(Pos2::new(r.left(), r.top() + k as f32 * 15.0), Vec2::new(22.0, 15.0));
            let resp = ui.interact(half, base.with(("step", k)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(half.shrink(1.0), CornerRadius::same(3), BG4);
            }
            let icon = if k == 0 { egui_phosphor::regular::CARET_UP } else { egui_phosphor::regular::CARET_DOWN };
            ui.painter().text(half.center(), Align2::CENTER_CENTER, icon, FontId::new(12.0, bold_icons()), if resp.hovered() { TEXT } else { TEXT_DIM });
            if resp.clicked() {
                let d = if k == 0 { n.step } else { -n.step };
                *v = (*v + d).clamp(*range.start(), *range.end());
                changed = true;
            }
        }
    });
    changed
}

/// Label · slider · value box. Double-click the label to reset to `default`.
pub fn slider_row(ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32, n: Num) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let l = ui
            .add_sized(Vec2::new(80.0, 28.0), egui::Label::new(egui::RichText::new(label).color(TEXT_DIM)).sense(Sense::click()))
            .on_hover_text("Double-click to reset");
        if l.double_clicked() && *v != default {
            *v = default;
            changed = true;
        }
        // Room for the value box (64) + steppers (22) + spacing.
        let w = ui.available_width() - 64.0 - 22.0 - 4.0 - ui.spacing().item_spacing.x * 2.0 - 4.0;
        changed |= slider(ui, v, range.clone(), w).changed();
        changed |= value_box(ui, v, range, n);
    });
    changed
}

/// Play glyph, optically centred (centroid on `c`).
pub fn paint_play(p: &egui::Painter, c: Pos2, size: f32, col: Color32) {
    let w = size * 0.86;
    let h = size;
    let x1 = c.x - w / 3.0;
    let x2 = c.x + 2.0 * w / 3.0;
    let pts = vec![Pos2::new(x1, c.y - h / 2.0), Pos2::new(x2, c.y), Pos2::new(x1, c.y + h / 2.0)];
    p.add(Shape::convex_polygon(pts, col, Stroke::new(1.6, col)));
}

pub fn paint_pause(p: &egui::Painter, c: Pos2, size: f32, col: Color32) {
    let bw = size * 0.3;
    let gap = size * 0.24;
    for dx in [-(gap / 2.0 + bw / 2.0), gap / 2.0 + bw / 2.0] {
        p.rect_filled(Rect::from_center_size(c + Vec2::new(dx, 0.0), Vec2::new(bw, size)), CornerRadius::same(2), col);
    }
}

// --------------------------------------------------------------------- logo

/// Klip mark geometry in unit space: a "K" built from three capsules —
/// the stem is the playhead, the arms are two clips meeting at a cut.
/// Returns (from, to, radius, alpha).
fn logo_strokes() -> [((f32, f32), (f32, f32), f32, f32); 3] {
    [
        ((0.33, 0.27), (0.33, 0.73), 0.085, 1.0),
        ((0.47, 0.50), (0.67, 0.29), 0.085, 1.0),
        ((0.53, 0.55), (0.68, 0.72), 0.085, 0.62),
    ]
}

const LOGO_TILE: Color32 = Color32::from_rgb(0x6f, 0x9b, 0xe2);

pub fn paint_logo(p: &egui::Painter, r: Rect) {
    let s = r.width();
    p.rect_filled(r, CornerRadius::same((0.26 * s) as u8), LOGO_TILE);
    let at = |(x, y): (f32, f32)| r.min + Vec2::new(x * s, y * s);
    for (a, b, rad, alpha) in logo_strokes() {
        let col = Color32::from_white_alpha((alpha * 255.0) as u8);
        let (a, b, w) = (at(a), at(b), rad * s);
        p.line_segment([a, b], Stroke::new(w * 2.0, col));
        p.circle_filled(a, w, col);
        p.circle_filled(b, w, col);
    }
}

/// The same mark rasterised for the window/taskbar icon.
pub fn app_icon() -> egui::IconData {
    const N: usize = 128;
    let s = N as f32;
    let mut rgba = vec![0u8; N * N * 4];
    let m = 0.06 * s;
    let inner = s - 2.0 * m;
    let rr_cov = |px: f32, py: f32| -> f32 {
        let (x, y, w, h, r) = (m, m, inner, inner, 0.26 * inner);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let qx = (px - cx).abs() - (w / 2.0 - r);
        let qy = (py - cy).abs() - (h / 2.0 - r);
        let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
        (0.5 - d).clamp(0.0, 1.0)
    };
    let capsule = |px: f32, py: f32, a: (f32, f32), b: (f32, f32), r: f32| -> f32 {
        let (ax, ay) = (m + a.0 * inner, m + a.1 * inner);
        let (bx, by) = (m + b.0 * inner, m + b.1 * inner);
        let (pax, pay, bax, bay) = (px - ax, py - ay, bx - ax, by - ay);
        let t = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
        let d = ((pax - bax * t).powi(2) + (pay - bay * t).powi(2)).sqrt() - r * inner;
        (0.5 - d).clamp(0.0, 1.0)
    };
    for y in 0..N {
        for x in 0..N {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let tile = rr_cov(px, py);
            if tile <= 0.0 {
                continue;
            }
            let base = LOGO_TILE.to_array();
            let mut c = [base[0] as f32, base[1] as f32, base[2] as f32];
            for (a, b, rad, alpha) in logo_strokes() {
                let k = capsule(px, py, a, b, rad) * alpha;
                for ch in &mut c {
                    *ch = *ch * (1.0 - k) + 255.0 * k;
                }
            }
            let o = (y * N + x) * 4;
            for i in 0..3 {
                rgba[o + i] = c[i].round() as u8;
            }
            rgba[o + 3] = (tile * 255.0).round() as u8;
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}
