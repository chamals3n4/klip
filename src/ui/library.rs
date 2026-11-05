//! Left panel: media library, audio, text presets, captions.

use crate::app::{KlipApp, LeftTab, MediaDrag, caption_template, to_image};
use crate::fx::{FilterPreset, TransitionKind};
use crate::model::{MediaKind, TextClip, TextStyle, timecode};
use crate::theme::*;
use egui::{Align2, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;

pub fn library(app: &mut KlipApp, ui: &mut egui::Ui) {
    tabs(app, ui);
    let rest = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rest.shrink2(Vec2::new(14.0, 10.0))));
    child.set_opacity(fade_in(ui, "lib_tab", app.left_tab as u64));
    egui::ScrollArea::vertical().auto_shrink(false).show(&mut child, |ui| match app.left_tab {
        LeftTab::Media => media_tab(app, ui),
        LeftTab::Audio => audio_tab(app, ui),
        LeftTab::Text => text_tab(app, ui),
        LeftTab::Captions => captions_tab(app, ui),
        LeftTab::Transitions => transitions_tab(app, ui),
        LeftTab::Filters => filters_tab(app, ui),
    });
}

fn tabs(app: &mut KlipApp, ui: &mut egui::Ui) {
    let tabs = [
        (LeftTab::Media, ph::FILM_STRIP, "Media"),
        (LeftTab::Audio, ph::MUSIC_NOTES, "Audio"),
        (LeftTab::Text, ph::TEXT_T, "Text"),
        (LeftTab::Captions, ph::SUBTITLES, "Captions"),
        (LeftTab::Transitions, ph::ARROWS_LEFT_RIGHT, "Transitions"),
        (LeftTab::Filters, ph::SWATCHES, "Filters"),
    ];
    let full = ui.available_rect_before_wrap();
    let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), 58.0));
    ui.allocate_rect(bar, Sense::hover());
    let w = (bar.width() - 12.0) / tabs.len() as f32;
    for (i, (tab, icon, label)) in tabs.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(bar.left() + 6.0 + w * i as f32, bar.top() + 6.0), Vec2::new(w, 48.0));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        let sel = app.left_tab == *tab;
        let h = anim(ui, ("tab_h", i), resp.hovered());
        let t = anim(ui, ("tab_s", i), sel);
        let col = mix(mix(TEXT_DIM, TEXT, h), TEXT, t);
        let p = ui.painter();
        let bg = mix(mix(Color32::TRANSPARENT, BG2, h), BG3, t);
        if bg.a() > 0 {
            p.rect_filled(r.shrink(3.0), CornerRadius::same(R_SM + 1), bg);
        }
        p.text(r.center() - Vec2::new(0.0, 8.0), Align2::CENTER_CENTER, *icon, FontId::proportional(19.0), mix(col, ACCENT, t));
        p.text(r.center() + Vec2::new(0.0, 13.0), Align2::CENTER_CENTER, *label, FontId::proportional(11.0), col);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.left_tab = *tab;
        }
    }
    ui.painter().hline(bar.x_range().shrink(10.0), bar.bottom(), Stroke::new(1.0, BORDER));
}

fn header(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
    ui.add_space(2.0);
}

fn drop_zone(app: &mut KlipApp, ui: &mut egui::Ui, title: &str, sub: &str) {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::click());
    let hov = anim(ui, ("drop", title), resp.hovered());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(R_MD), mix(BG2, BG3, hov));
    // dashed border
    let stroke = Stroke::new(1.2, mix(BG4, ACCENT, hov));
    dashed_rrect(p, r.shrink(0.5), R_MD as f32, stroke);
    let c = r.center();
    p.rect_filled(Rect::from_center_size(c - Vec2::new(0.0, 22.0), Vec2::splat(40.0)), CornerRadius::same(R_MD), mix(ACCENT, ACCENT_HI, hov));
    p.text(c - Vec2::new(0.0, 22.0), Align2::CENTER_CENTER, ph::PLUS, FontId::proportional(20.0), ON_ACCENT);
    p.text(c + Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, title, FontId::proportional(14.0), TEXT);
    p.text(c + Vec2::new(0.0, 36.0), Align2::CENTER_CENTER, sub, FontId::proportional(11.5), TEXT_DIM);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.dialog_import();
    }
}

fn media_tab(app: &mut KlipApp, ui: &mut egui::Ui) {
    header(ui, "Media", |ui| {
        if secondary_button(ui, format!("{}  Import", ph::PLUS)).on_hover_text("Import media (Ctrl+I)").clicked() {
            app.dialog_import();
        }
    });
    let items: Vec<_> = app.project.media.iter().filter(|m| m.kind != MediaKind::Audio).cloned().collect();
    if items.is_empty() {
        drop_zone(app, ui, "Import videos & photos", "or drag files anywhere into Klip");
        ui.add_space(10.0);
        tip(ui, ph::LIGHTBULB, "Tip: double-click a clip to add it, or drag it onto the timeline.");
        return;
    }
    let cols = 2;
    let gap = 10.0;
    let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let thumb_h = (w * 0.72).floor();
    let h = thumb_h + 24.0;
    for row in items.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for m in row {
                media_tile(app, ui, m, Vec2::new(w, h), thumb_h);
            }
        });
        ui.add_space(4.0);
    }
}

fn media_tile(app: &mut KlipApp, ui: &mut egui::Ui, m: &crate::model::MediaInfo, size: Vec2, thumb_h: f32) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let id = m.id;
    let thumb_r = Rect::from_min_size(rect.min, Vec2::new(size.x, thumb_h));
    let hov_b = resp.hovered() || resp.dragged();
    let hov = anim(ui, ("tile_h", id), hov_b);
    let sel_t = anim(ui, ("tile_s", id), app.lib_selected == Some(id));
    let sel = app.lib_selected == Some(id);
    let used = app.project.video.iter().any(|c| c.media == id);
    paint_thumb(ui, app, id, thumb_r, m.aspect());
    let p = ui.painter();
    // Duration / kind badge.
    let badge = if m.kind == MediaKind::Image { ph::IMAGE.to_string() } else { short_dur(m.duration) };
    let g = p.layout_no_wrap(badge, FontId::proportional(11.0), Color32::WHITE);
    let br = Rect::from_min_size(thumb_r.right_bottom() - g.size() - Vec2::new(12.0, 10.0), g.size() + Vec2::new(8.0, 4.0));
    p.rect_filled(br, CornerRadius::same(4), Color32::from_black_alpha(170));
    p.galley(br.min + Vec2::new(4.0, 2.0), g, Color32::WHITE);
    if used {
        let c = thumb_r.left_top() + Vec2::new(14.0, 14.0);
        p.circle_filled(c, 8.0, Color32::from_black_alpha(160));
        p.text(c, Align2::CENTER_CENTER, ph::CHECK, FontId::proportional(10.0), SUCCESS);
    }
    let stroke_col = mix(mix(Color32::TRANSPARENT, BORDER_HI, hov), ACCENT, sel_t);
    if stroke_col.a() > 0 {
        p.rect_stroke(thumb_r, CornerRadius::same(R_MD), Stroke::new(1.0 + sel_t, stroke_col), StrokeKind::Inside);
    }
    p.text(
        Pos2::new(rect.left() + 2.0, thumb_r.bottom() + 12.0),
        Align2::LEFT_CENTER,
        elide(&m.name, size.x - 4.0, 7.0),
        FontId::proportional(11.5),
        if sel { TEXT } else { TEXT_DIM },
    );

    // Hover "+" button.
    let plus_c = thumb_r.right_top() + Vec2::new(-16.0, 16.0);
    let plus_r = Rect::from_center_size(plus_c, Vec2::splat(26.0));
    let plus = ui.interact(plus_r, ui.id().with(("add", id)), Sense::click());
    let show = anim(ui, ("plus_v", id), hov_b || plus.hovered());
    if show > 0.01 {
        let ph_t = anim(ui, ("plus_h", id), plus.hovered());
        let p = ui.painter();
        p.rect_filled(plus_r, CornerRadius::same(R_SM), mix(ACCENT, ACCENT_HI, ph_t).gamma_multiply(show));
        p.text(plus_c, Align2::CENTER_CENTER, ph::PLUS, FontId::proportional(14.0), ON_ACCENT.gamma_multiply(show));
    }
    if plus.on_hover_text("Add to timeline").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.add_to_main(id, None);
    }
    if resp.hovered() && !resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), MediaDrag(id));
    }
    if resp.dragged() {
        // Ghost under the cursor.
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
            let gp = ui.ctx().layer_painter(layer);
            let gr = Rect::from_center_size(pos, Vec2::new(96.0, 96.0 / m.aspect().max(0.5).min(2.0)));
            if let Some(t) = app.assets.get(&id).and_then(|a| a.thumb.as_ref()) {
                gp.image(t.id(), gr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha(220));
            } else {
                gp.rect_filled(gr, CornerRadius::same(6), BG4);
            }
            gp.rect_stroke(gr, CornerRadius::same(6), Stroke::new(2.0, ACCENT), StrokeKind::Outside);
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if resp.clicked() {
        app.lib_selected = Some(id);
    }
    if resp.double_clicked() {
        app.add_to_main(id, None);
    }
    resp.on_hover_text(format!("{}\n{}×{}", m.name, m.width, m.height)).context_menu(|ui| {
        if ui.button(format!("{} Add to timeline", ph::PLUS)).clicked() {
            app.add_to_main(id, None);
            ui.close();
        }
        if ui.button(format!("{} Remove from project", ph::TRASH)).clicked() {
            app.remove_media(id);
            ui.close();
        }
    });
}

/// Thumbnail cropped to cover `r`.
fn paint_thumb(ui: &egui::Ui, app: &KlipApp, id: crate::model::Id, r: Rect, aspect: f32) {
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(R_MD), BG2);
    if let Some(t) = app.assets.get(&id).and_then(|a| a.thumb.as_ref()) {
        let uv = cover_uv(aspect, r.aspect_ratio());
        // Paint inset by 1px so the rounded border hides the square corners.
        p.image(t.id(), r, uv, Color32::WHITE);
        // Mask corners with the panel colour for a rounded look.
        round_mask(p, r, R_MD as f32, BG1);
    } else {
        p.text(r.center(), Align2::CENTER_CENTER, ph::SPINNER_GAP, FontId::proportional(18.0), TEXT_FAINT);
    }
}

/// UV rect that crops an image of `src_aspect` to fill `dst_aspect`.
pub fn cover_uv(src_aspect: f32, dst_aspect: f32) -> Rect {
    if src_aspect > dst_aspect {
        let f = dst_aspect / src_aspect;
        Rect::from_min_max(Pos2::new(0.5 - f / 2.0, 0.0), Pos2::new(0.5 + f / 2.0, 1.0))
    } else {
        let f = src_aspect / dst_aspect;
        Rect::from_min_max(Pos2::new(0.0, 0.5 - f / 2.0), Pos2::new(1.0, 0.5 + f / 2.0))
    }
}

/// Paint background-coloured corner wedges so square images look rounded.
pub fn round_mask(p: &egui::Painter, r: Rect, rad: f32, bg: Color32) {
    let n = 6;
    for (corner, sx, sy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.right_bottom(), -1.0, -1.0),
        (r.left_bottom(), 1.0, -1.0),
    ] {
        let c = corner + Vec2::new(sx * rad, sy * rad);
        let mut pts = vec![corner];
        for i in 0..=n {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / n as f32;
            pts.push(c + Vec2::new(-sx * rad * a.cos(), -sy * rad * a.sin()));
        }
        pts.reverse();
        p.add(egui::Shape::convex_polygon(pts, bg, Stroke::NONE));
    }
}

fn audio_tab(app: &mut KlipApp, ui: &mut egui::Ui) {
    header(ui, "Audio", |ui| {
        if secondary_button(ui, format!("{}  Import", ph::PLUS)).clicked() {
            app.dialog_import();
        }
    });
    let items: Vec<_> = app.project.media.iter().filter(|m| m.has_audio).cloned().collect();
    if items.is_empty() {
        drop_zone(app, ui, "Import music & sound", "MP3, WAV, M4A, FLAC…");
        return;
    }
    tip(ui, ph::INFO, "Adds to the music track at the playhead. Video files can be used too.");
    ui.add_space(4.0);
    for m in items {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click_and_drag());
        let hov = anim(ui, ("arow", m.id), resp.hovered());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(R_MD), mix(BG2, BG3, hov));
        let ic = Rect::from_min_size(r.min + Vec2::new(8.0, 8.0), Vec2::splat(36.0));
        p.rect_filled(ic, CornerRadius::same(R_SM + 1), ACCENT.gamma_multiply(0.14));
        let icon = if m.kind == MediaKind::Audio { ph::MUSIC_NOTE } else { ph::FILM_SLATE };
        p.text(ic.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), ACCENT);
        p.text(ic.right_center() + Vec2::new(10.0, -8.0), Align2::LEFT_CENTER, elide(&m.name, r.width() - 110.0, 7.0), FontId::proportional(13.0), TEXT);
        p.text(ic.right_center() + Vec2::new(10.0, 10.0), Align2::LEFT_CENTER, timecode(m.duration), FontId::proportional(11.0), TEXT_DIM);
        // mini waveform
        if let Some(w) = app.assets.get(&m.id).and_then(|a| a.wave.clone()) {
            let wr = Rect::from_min_max(Pos2::new(r.right() - 74.0, r.top() + 14.0), Pos2::new(r.right() - 40.0, r.bottom() - 14.0));
            let n = 12;
            for i in 0..n {
                let v = w.get(i * w.len() / n).copied().unwrap_or(0.0);
                let x = wr.left() + i as f32 * wr.width() / n as f32;
                let hh = (v * wr.height()).max(2.0);
                p.rect_filled(Rect::from_center_size(Pos2::new(x, wr.center().y), Vec2::new(1.6, hh)), CornerRadius::ZERO, TEXT_FAINT);
            }
        }
        let plus_c = Pos2::new(r.right() - 20.0, r.center().y);
        let plus = ui.interact(Rect::from_center_size(plus_c, Vec2::splat(26.0)), ui.id().with(("addaudio", m.id)), Sense::click());
        let pt = anim(ui, ("aplus", m.id), plus.hovered());
        ui.painter().rect_filled(Rect::from_center_size(plus_c, Vec2::splat(26.0)), CornerRadius::same(R_SM), mix(ACCENT, ACCENT_HI, pt));
        ui.painter().text(plus_c, Align2::CENTER_CENTER, ph::PLUS, FontId::proportional(14.0), ON_ACCENT);
        if plus.on_hover_text("Add at playhead").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() || resp.double_clicked() {
            app.add_audio(m.id, app.playhead);
        }
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), crate::app::AudioDrag(m.id));
        }
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        ui.add_space(2.0);
    }
}

pub fn text_presets() -> Vec<(&'static str, TextClip)> {
    let base = TextClip {
        id: 0,
        text: String::new(),
        start: 0.0,
        dur: 3.0,
        x: 0.5,
        y: 0.5,
        size: 80.0,
        color: [255, 255, 255],
        style: TextStyle::Plain,
        is_caption: false,
    };
    vec![
        ("Title", TextClip { text: "Your title".into(), size: 104.0, y: 0.32, style: TextStyle::Shadow, ..base.clone() }),
        ("Hook", TextClip { text: "WAIT FOR IT".into(), size: 92.0, y: 0.2, color: [255, 214, 10], style: TextStyle::Outline, ..base.clone() }),
        ("Label", TextClip { text: "Add text".into(), size: 58.0, y: 0.8, style: TextStyle::Boxed, ..base.clone() }),
        ("Clean", TextClip { text: "Simple text".into(), size: 64.0, y: 0.5, ..base.clone() }),
        ("Pop", TextClip { text: "POV:".into(), size: 120.0, y: 0.42, color: [255, 79, 154], style: TextStyle::Outline, ..base.clone() }),
        ("Note", TextClip { text: "link in bio".into(), size: 48.0, y: 0.88, color: [20, 20, 24], style: TextStyle::Plain, ..base }),
    ]
}

fn text_tab(app: &mut KlipApp, ui: &mut egui::Ui) {
    header(ui, "Text", |_| {});
    if primary_button(ui, format!("{}  Add text", ph::TEXT_T)).clicked() {
        let mut t = text_presets()[3].1.clone();
        t.text = "Add text".into();
        app.add_text(t);
    }
    ui.add_space(8.0);
    section(ui, "Styles");
    let presets = text_presets();
    let gap = 10.0;
    let w = ((ui.available_width() - gap) / 2.0).floor();
    for row in presets.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (name, t) in row {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 78.0), Sense::click());
                let hov = anim(ui, ("preset", *name), resp.hovered());
                let p = ui.painter();
                let bg = if t.color == [20, 20, 24] { Color32::from_rgb(0xf2, 0xf2, 0xf5) } else { BG2 };
                p.rect_filled(r, CornerRadius::same(R_MD), if bg == BG2 { mix(BG2, BG3, hov) } else { bg });
                if hov > 0.01 {
                    p.rect_stroke(r, CornerRadius::same(R_MD), Stroke::new(1.5, ACCENT.gamma_multiply(hov)), StrokeKind::Inside);
                }
                let scale = (w * 0.8 / (t.text.chars().count() as f32 * t.size * 0.62)).min(0.3);
                super::preview::paint_text(p, t, r.center() - Vec2::new(0.0, 6.0), scale);
                p.text(r.center_bottom() - Vec2::new(0.0, 10.0), Align2::CENTER_CENTER, *name, FontId::proportional(10.5), if bg == BG2 { TEXT_DIM } else { BG3 });
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.add_text(t.clone());
                }
            }
        });
        ui.add_space(4.0);
    }
}

fn captions_tab(app: &mut KlipApp, ui: &mut egui::Ui) {
    header(ui, "Captions", |_| {});
    let n = app.project.texts.iter().filter(|t| t.is_caption).count();

    egui::Frame::new()
        .fill(BG2)
        .corner_radius(CornerRadius::same(R_MD))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Auto captions").family(medium_family()).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::Frame::new()
                        .fill(BG3)
                        .corner_radius(CornerRadius::same(R_SM))
                        .inner_margin(Margin::symmetric(8, 3))
                        .show(ui, |ui| ui.label(RichText::new("Available soon").size(11.5).color(TEXT_DIM)));
                });
            });
            ui.label(RichText::new("Generate captions from speech automatically.").size(12.5).color(TEXT_DIM));
        });
    ui.add_space(10.0);
    if primary_button(ui, format!("{}  Import .srt subtitles", ph::FILE_ARROW_UP)).clicked() {
        app.dialog_import_srt();
    }
    ui.add_space(14.0);

    if n == 0 {
        return;
    }
    section(ui, &format!("Style all captions · {n}"));
    let mut tmpl = app.project.texts.iter().find(|t| t.is_caption).cloned().unwrap_or_else(|| caption_template(0));
    let before = tmpl.clone();
    let opts: Vec<_> = TextStyle::ALL.iter().map(|s| (*s, s.label())).collect();
    segmented(ui, &mut tmpl.style, &opts);
    slider_row(ui, "Size", &mut tmpl.size, 24.0..=160.0, 66.0, num("", 0, 1.0));
    let mut y = tmpl.y * 100.0;
    if slider_row(ui, "Position", &mut y, 5.0..=95.0, 72.0, num("%", 0, 1.0)) {
        tmpl.y = y / 100.0;
    }
    super::inspector::color_swatches(ui, &mut tmpl.color);
    if tmpl != before {
        for t in app.project.texts.iter_mut().filter(|t| t.is_caption) {
            t.style = tmpl.style;
            t.size = tmpl.size;
            t.y = tmpl.y;
            t.color = tmpl.color;
        }
    }
    ui.add_space(12.0);
    if ui.button(RichText::new(format!("{}  Remove all captions", ph::TRASH)).color(DANGER)).clicked() {
        app.project.texts.retain(|t| !t.is_caption);
    }
}

pub fn tip(ui: &mut egui::Ui, icon: &str, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(icon).color(TEXT_FAINT));
        ui.label(RichText::new(text).size(11.5).color(TEXT_FAINT));
    });
}

pub fn short_dur(s: f64) -> String {
    let s = s.round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn elide(s: &str, width: f32, char_w: f32) -> String {
    let max = (width / char_w).max(3.0) as usize;
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

const DEMO_A: Color32 = Color32::from_rgb(0x4f, 0x74, 0xb8);
const DEMO_B: Color32 = Color32::from_rgb(0xd9, 0x8a, 0x5b);

/// Mini animation of a transition from frame A to frame B at progress `q`.
fn paint_transition_demo(p: &egui::Painter, r: Rect, kind: TransitionKind, q: f32) {
    let p = p.with_clip_rect(r);
    let frame = |p: &egui::Painter, rr: Rect, col: Color32, label: &str, alpha: f32| {
        p.rect_filled(rr, CornerRadius::ZERO, col.gamma_multiply(alpha));
        p.text(rr.center(), Align2::CENTER_CENTER, label, FontId::new(15.0, semibold_family()), Color32::from_white_alpha((200.0 * alpha) as u8));
    };
    let (w, h) = (r.width(), r.height());
    let ease = |x: f32| x * x * (3.0 - 2.0 * x);
    match kind {
        TransitionKind::Mix => {
            frame(&p, r, DEMO_A, "A", 1.0);
            frame(&p, r, DEMO_B, "B", q);
        }
        TransitionKind::FadeBlack | TransitionKind::FadeWhite => {
            let c = if kind == TransitionKind::FadeBlack { 0 } else { 255 };
            let (col, l, k) = if q < 0.5 { (DEMO_A, "A", q * 2.0) } else { (DEMO_B, "B", 2.0 - q * 2.0) };
            frame(&p, r, col, l, 1.0);
            p.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(c, c, c, (ease(k) * 255.0) as u8));
        }
        TransitionKind::SlideLeft => {
            frame(&p, r.translate(Vec2::new(-q * w, 0.0)), DEMO_A, "A", 1.0);
            frame(&p, r.translate(Vec2::new((1.0 - q) * w, 0.0)), DEMO_B, "B", 1.0);
        }
        TransitionKind::SlideRight => {
            frame(&p, r.translate(Vec2::new(q * w, 0.0)), DEMO_A, "A", 1.0);
            frame(&p, r.translate(Vec2::new(-(1.0 - q) * w, 0.0)), DEMO_B, "B", 1.0);
        }
        TransitionKind::SlideUp => {
            frame(&p, r.translate(Vec2::new(0.0, -q * h)), DEMO_A, "A", 1.0);
            frame(&p, r.translate(Vec2::new(0.0, (1.0 - q) * h)), DEMO_B, "B", 1.0);
        }
        TransitionKind::Wipe => {
            frame(&p, r, DEMO_A, "A", 1.0);
            let wr = Rect::from_min_max(Pos2::new(r.right() - q * w, r.top()), r.max);
            frame(&p.with_clip_rect(wr), r, DEMO_B, "B", 1.0);
        }
        TransitionKind::ZoomIn => {
            frame(&p, r, DEMO_B, "B", 1.0);
            let z = 1.0 + ease(q) * 0.6;
            frame(&p, Rect::from_center_size(r.center(), r.size() * z), DEMO_A, "A", 1.0 - ease(q));
        }
        TransitionKind::Blur => {
            frame(&p, r, DEMO_A, "A", 1.0);
            frame(&p, r, DEMO_B, "B", q);
            let haze = (std::f32::consts::PI * q).sin();
            p.rect_filled(r, CornerRadius::ZERO, Color32::from_white_alpha((haze * 70.0) as u8));
        }
    }
}

/// Grid of transition tiles (animated on hover). Returns the clicked kind.
pub fn transition_grid(ui: &mut egui::Ui, current: Option<TransitionKind>, cols: usize) -> Option<TransitionKind> {
    let gap = 8.0;
    let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let th = (w * 0.62).floor();
    let mut clicked = None;
    for row in TransitionKind::ALL.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for kind in row {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, th + 22.0), Sense::click());
                let tile = Rect::from_min_size(r.min, Vec2::new(w, th));
                let sel = current == Some(*kind);
                let h = anim(ui, ("tr_h", kind.label()), resp.hovered());
                let s = anim(ui, ("tr_s", kind.label()), sel);
                let q = if resp.hovered() {
                    ui.ctx().request_repaint();
                    let t = (ui.input(|i| i.time) * 0.8) as f32 % 1.5;
                    (t - 0.25).clamp(0.0, 1.0)
                } else {
                    0.45
                };
                paint_transition_demo(ui.painter(), tile, *kind, q);
                round_mask(ui.painter(), tile, R_SM as f32, BG1);
                let stroke = mix(mix(Color32::TRANSPARENT, BORDER_HI, h), ACCENT, s);
                if stroke.a() > 0 {
                    ui.painter().rect_stroke(tile, CornerRadius::same(R_SM), Stroke::new(1.0 + s, stroke), StrokeKind::Inside);
                }
                ui.painter().text(
                    Pos2::new(tile.center().x, tile.bottom() + 11.0),
                    Align2::CENTER_CENTER,
                    kind.label(),
                    FontId::proportional(11.5),
                    mix(TEXT_DIM, TEXT, s.max(h)),
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    clicked = Some(*kind);
                }
            }
        });
        ui.add_space(4.0);
    }
    clicked
}

fn transitions_tab(app: &mut KlipApp, ui: &mut egui::Ui) {
    header(ui, "Transitions", |_| {});
    let Some(i) = app.target_cut() else {
        ui.label(RichText::new("Add two or more clips to the timeline to use transitions.").size(12.5).color(TEXT_DIM));
        return;
    };
    let name = |k: usize| {
        app.project.video.get(k).and_then(|c| app.project.media(c.media)).map(|m| m.name.clone()).unwrap_or_default()
    };
    ui.label(
        RichText::new(format!("Cut between {} and {}", elide(&name(i), 110.0, 7.0), elide(&name(i + 1), 110.0, 7.0)))
            .size(12.0)
            .color(TEXT_DIM),
    );
    ui.add_space(6.0);
    let current = app.project.video[i].transition.map(|t| t.kind);
    if let Some(k) = transition_grid(ui, current, 3) {
        app.apply_transition(k);
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

fn filters_tab(app: &mut KlipApp, ui: &mut egui::Ui) {
    header(ui, "Filters", |_| {});
    let Some(ci) = app.target_clip() else {
        ui.label(RichText::new("Add a clip to the timeline to use filters.").size(12.5).color(TEXT_DIM));
        return;
    };
    let clip = app.project.video[ci].clone();
    let aspect = app.project.media(clip.media).map(|m| m.aspect()).unwrap_or(1.0);

    // Build graded thumbnails for this clip's media once.
    if let Some(a) = app.assets.get_mut(&clip.media) {
        if let Some(raw) = a.thumb_raw.clone() {
            for preset in FilterPreset::ALL {
                a.filtered.entry(preset).or_insert_with(|| {
                    let mut f = raw.clone();
                    preset.look().apply(&mut f.rgba);
                    ui.ctx().load_texture(format!("filter{}_{preset:?}", clip.media), to_image(&f), egui::TextureOptions::LINEAR)
                });
            }
        }
    }

    let cols = 3;
    let gap = 8.0;
    let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let th = w.floor();
    let mut picked = None;
    for row in FilterPreset::ALL.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for preset in row {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, th + 22.0), Sense::click());
                let tile = Rect::from_min_size(r.min, Vec2::new(w, th));
                let sel = clip.filter == *preset;
                let h = anim(ui, ("flt_h", preset.label()), resp.hovered());
                let s = anim(ui, ("flt_s", preset.label()), sel);
                let p = ui.painter();
                p.rect_filled(tile, CornerRadius::same(R_SM), BG2);
                if let Some(t) = app.assets.get(&clip.media).and_then(|a| a.filtered.get(preset)) {
                    p.image(t.id(), tile, cover_uv(aspect, 1.0), Color32::WHITE);
                    round_mask(p, tile, R_SM as f32, BG1);
                }
                let stroke = mix(mix(Color32::TRANSPARENT, BORDER_HI, h), ACCENT, s);
                if stroke.a() > 0 {
                    p.rect_stroke(tile, CornerRadius::same(R_SM), Stroke::new(1.0 + s, stroke), StrokeKind::Inside);
                }
                p.text(
                    Pos2::new(tile.center().x, tile.bottom() + 11.0),
                    Align2::CENTER_CENTER,
                    preset.label(),
                    FontId::proportional(11.5),
                    mix(TEXT_DIM, TEXT, s.max(h)),
                );
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    picked = Some(*preset);
                }
            }
        });
        ui.add_space(4.0);
    }
    if let Some(p) = picked {
        app.apply_filter(p);
    }
    let c = &mut app.project.video[ci];
    if c.filter != FilterPreset::None {
        ui.add_space(6.0);
        let mut amt = c.filter_amount * 100.0;
        if slider_row(ui, "Strength", &mut amt, 0.0..=100.0, 100.0, num("%", 0, 1.0)) {
            c.filter_amount = amt / 100.0;
        }
        ui.add_space(4.0);
        if link_button(ui, "Apply to all clips").clicked() {
            let (f, amt) = (c.filter, c.filter_amount);
            for o in app.project.video.iter_mut() {
                o.filter = f;
                o.filter_amount = amt;
            }
        }
    }
}
