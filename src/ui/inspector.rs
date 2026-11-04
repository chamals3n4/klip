//! Right panel: properties of the selected clip, or project info.

use crate::app::{KlipApp, Selection};
use crate::model::{Aspect, FitMode, MediaKind, TextStyle, timecode};
use crate::theme::*;
use egui::{Color32, CornerRadius, Margin, RichText, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;

pub fn inspector(app: &mut KlipApp, ui: &mut egui::Ui) {
    let (icon, title) = match app.selection {
        Selection::Video(_) => (ph::FILM_STRIP, "Video"),
        Selection::Text(id) => {
            if app.project.texts.iter().any(|t| t.id == id && t.is_caption) {
                (ph::SUBTITLES, "Caption")
            } else {
                (ph::TEXT_T, "Text")
            }
        }
        Selection::Audio(_) => (ph::MUSIC_NOTES, "Audio"),
        Selection::None => (ph::INFO, "Details"),
    };
    egui::Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon).size(17.0).color(ACCENT));
            ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
        });
    });
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range().shrink(10.0), r.top() - 2.0, Stroke::new(1.0, BORDER));

    let key = match app.selection {
        Selection::None => 0,
        Selection::Video(id) | Selection::Text(id) | Selection::Audio(id) => id,
    };
    ui.set_opacity(fade_in(ui, "inspector", key));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match app.selection {
                Selection::Video(id) => video_props(app, ui, id),
                Selection::Text(id) => text_props(app, ui, id),
                Selection::Audio(id) => audio_props(app, ui, id),
                Selection::None => project_props(app, ui),
            }
        });
    });
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(72.0, 22.0), egui::Label::new(RichText::new(label).color(TEXT_DIM)));
        add(ui);
    });
}

fn actions(app: &mut KlipApp, ui: &mut egui::Ui) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if secondary_button(ui, format!("{}  Split", ph::SCISSORS)).on_hover_text("Split at playhead (S)").clicked() {
            app.split();
        }
        if secondary_button(ui, format!("{}  Duplicate", ph::COPY)).on_hover_text("Ctrl+D").clicked() {
            app.duplicate_selection();
        }
        if ui.add(egui::Button::new(RichText::new(ph::TRASH).size(16.0).color(DANGER)).min_size(Vec2::splat(36.0))).on_hover_text("Delete (Del)").clicked() {
            app.delete_selection();
        }
    });
}

fn video_props(app: &mut KlipApp, ui: &mut egui::Ui, id: u64) {
    let Some(i) = app.project.video_index(id) else { return };
    let media = app.project.media(app.project.video[i].media).cloned();
    if let Some(m) = &media {
        ui.label(RichText::new(&m.name).color(TEXT).strong());
        ui.label(
            RichText::new(match m.kind {
                MediaKind::Image => format!("Photo · {}×{}", m.width, m.height),
                _ => format!("{}×{} · {}", m.width, m.height, timecode(m.duration)),
            })
            .size(11.5)
            .color(TEXT_DIM),
        );
    }
    ui.add_space(10.0);
    section(ui, "Canvas");
    let before = app.project.video[i].fit;
    fit_picker(ui, &mut app.project.video[i].fit);
    if app.project.video[i].fit != before {
        app.invalidate_preview();
    }
    ui.add_space(4.0);
    if ui.add(egui::Button::new(RichText::new("Apply to all clips").size(12.5).color(ACCENT)).frame_when_inactive(false)).clicked() {
        let f = app.project.video[i].fit;
        app.project.video.iter_mut().for_each(|c| c.fit = f);
    }

    ui.add_space(10.0);
    section(ui, "Timing");
    let is_image = media.as_ref().is_some_and(|m| m.kind == MediaKind::Image);
    let max = media.as_ref().map(|m| if is_image { 3600.0 } else { m.duration }).unwrap_or(0.0);
    let c = &mut app.project.video[i];
    let mut dur = c.dur();
    row(ui, "Duration", |ui| {
        if ui.add(egui::DragValue::new(&mut dur).range(0.1..=(max - c.src_in).max(0.1)).speed(0.05).suffix(" s").max_decimals(2)).changed() {
            c.src_out = c.src_in + dur;
        }
    });
    if !is_image {
        let src_out = c.src_out;
        row(ui, "Trim in", |ui| {
            ui.add(egui::DragValue::new(&mut c.src_in).range(0.0..=(src_out - 0.1)).speed(0.05).suffix(" s").max_decimals(2));
        });
    }

    if media.as_ref().is_some_and(|m| m.has_audio) {
        ui.add_space(10.0);
        section(ui, "Audio");
        let c = &mut app.project.video[i];
        volume_row(ui, &mut c.volume);
    }
    actions(app, ui);
}

fn fit_picker(ui: &mut egui::Ui, fit: &mut FitMode) {
    let w = (ui.available_width() - 16.0) / 3.0;
    ui.horizontal(|ui| {
        for f in FitMode::ALL {
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 64.0), Sense::click());
            let sel = *fit == f;
            let h = anim(ui, ("fit_h", f.label()), resp.hovered());
            let t = anim(ui, ("fit_s", f.label()), sel);
            let p = ui.painter();
            p.rect_filled(r, CornerRadius::same(R_MD), mix(mix(BG0, BG2, h), BG3, t));
            p.rect_stroke(r, CornerRadius::same(R_MD), Stroke::new(1.5, mix(BORDER, ACCENT, t)), StrokeKind::Inside);
            // mini diagram: portrait frame with a landscape clip inside
            let frame = egui::Rect::from_center_size(r.center() - Vec2::new(0.0, 8.0), Vec2::new(18.0, 30.0));
            let clip_col = mix(TEXT_DIM, ACCENT, t);
            match f {
                FitMode::Fill => {
                    p.rect_filled(frame, CornerRadius::same(2), clip_col);
                }
                FitMode::Fit => {
                    p.rect_filled(frame, CornerRadius::same(2), Color32::BLACK);
                    p.rect_filled(egui::Rect::from_center_size(frame.center(), Vec2::new(18.0, 10.0)), CornerRadius::ZERO, clip_col);
                }
                FitMode::Blur => {
                    p.rect_filled(frame, CornerRadius::same(2), clip_col.gamma_multiply(0.35));
                    p.rect_filled(egui::Rect::from_center_size(frame.center(), Vec2::new(18.0, 10.0)), CornerRadius::ZERO, clip_col);
                }
            }
            p.rect_stroke(frame, CornerRadius::same(2), Stroke::new(1.0, TEXT_DIM), StrokeKind::Outside);
            p.text(r.center_bottom() - Vec2::new(0.0, 10.0), egui::Align2::CENTER_CENTER, f.label(), egui::FontId::proportional(12.0), mix(TEXT_DIM, TEXT, t));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *fit = f;
            }
        }
    });
}

fn volume_row(ui: &mut egui::Ui, v: &mut f32) {
    row(ui, "Volume", |ui| {
        let icon = if *v <= 0.001 { ph::SPEAKER_SLASH } else { ph::SPEAKER_HIGH };
        if ui.add(egui::Button::new(icon).frame(false)).on_hover_text("Mute").clicked() {
            *v = if *v <= 0.001 { 1.0 } else { 0.0 };
        }
        let mut pct = *v * 100.0;
        if ui.add(egui::Slider::new(&mut pct, 0.0..=200.0).suffix("%").max_decimals(0)).changed() {
            *v = pct / 100.0;
        }
    });
}

pub const SWATCHES: [[u8; 3]; 12] = [
    [255, 255, 255],
    [20, 20, 24],
    [255, 214, 10],
    [255, 159, 28],
    [255, 79, 154],
    [255, 59, 48],
    [124, 92, 255],
    [10, 132, 255],
    [50, 215, 255],
    [52, 199, 89],
    [191, 255, 0],
    [190, 170, 140],
];

pub fn color_swatches(ui: &mut egui::Ui, color: &mut [u8; 3]) {
    let size = 22.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(6.0);
        for c in SWATCHES {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
            let col = Color32::from_rgb(c[0], c[1], c[2]);
            let p = ui.painter();
            let sel = *color == c;
            if sel {
                p.circle_stroke(r.center(), size / 2.0, Stroke::new(2.0, ACCENT_HI));
            }
            p.circle_filled(r.center(), size / 2.0 - if sel { 4.0 } else { 1.0 }, col);
            if c == [20, 20, 24] {
                p.circle_stroke(r.center(), size / 2.0 - 1.0, Stroke::new(1.0, BG4));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *color = c;
            }
        }
        let mut c32 = Color32::from_rgb(color[0], color[1], color[2]);
        if egui::color_picker::color_edit_button_srgba(ui, &mut c32, egui::color_picker::Alpha::Opaque).changed() {
            *color = [c32.r(), c32.g(), c32.b()];
        }
    });
}

fn text_props(app: &mut KlipApp, ui: &mut egui::Ui, id: u64) {
    let Some(i) = app.project.texts.iter().position(|t| t.id == id) else { return };
    let total = app.project.duration();
    let t = &mut app.project.texts[i];
    let edit = egui::TextEdit::multiline(&mut t.text)
        .desired_rows(3)
        .desired_width(f32::INFINITY)
        .hint_text("Type something…")
        .margin(Margin::same(10))
        .background_color(BG0);
    ui.add(edit);
    ui.add_space(10.0);

    section(ui, "Style");
    let opts: Vec<_> = TextStyle::ALL.iter().map(|s| (*s, s.label())).collect();
    segmented(ui, &mut t.style, &opts);
    ui.add_space(6.0);
    row(ui, "Size", |ui| {
        ui.add(egui::Slider::new(&mut t.size, 16.0..=220.0).max_decimals(0));
    });
    ui.add_space(4.0);
    color_swatches(ui, &mut t.color);

    ui.add_space(10.0);
    section(ui, "Position");
    row(ui, "X", |ui| {
        ui.add(egui::Slider::new(&mut t.x, 0.0..=1.0).show_value(false));
    });
    row(ui, "Y", |ui| {
        ui.add(egui::Slider::new(&mut t.y, 0.0..=1.0).show_value(false));
    });
    ui.horizontal(|ui| {
        if ui.small_button("Top").clicked() {
            t.y = 0.18;
        }
        if ui.small_button("Center").clicked() {
            t.x = 0.5;
            t.y = 0.5;
        }
        if ui.small_button("Bottom").clicked() {
            t.y = 0.8;
        }
    });

    ui.add_space(10.0);
    section(ui, "Timing");
    row(ui, "Start", |ui| {
        ui.add(egui::DragValue::new(&mut t.start).range(0.0..=total.max(0.0)).speed(0.05).suffix(" s").max_decimals(2));
    });
    row(ui, "Duration", |ui| {
        ui.add(egui::DragValue::new(&mut t.dur).range(0.1..=600.0).speed(0.05).suffix(" s").max_decimals(2));
    });
    actions(app, ui);
}

fn audio_props(app: &mut KlipApp, ui: &mut egui::Ui, id: u64) {
    let Some(i) = app.project.audio.iter().position(|a| a.id == id) else { return };
    let media = app.project.media(app.project.audio[i].media).cloned();
    if let Some(m) = &media {
        ui.label(RichText::new(&m.name).color(TEXT).strong());
        ui.label(RichText::new(timecode(m.duration)).size(11.5).color(TEXT_DIM));
    }
    ui.add_space(10.0);
    section(ui, "Audio");
    let a = &mut app.project.audio[i];
    volume_row(ui, &mut a.volume);
    ui.add_space(10.0);
    section(ui, "Timing");
    let max = media.map(|m| m.duration).unwrap_or(0.0);
    row(ui, "Start", |ui| {
        ui.add(egui::DragValue::new(&mut a.start).range(0.0..=3600.0).speed(0.05).suffix(" s").max_decimals(2));
    });
    let src_in = a.src_in;
    row(ui, "Duration", |ui| {
        ui.add(egui::DragValue::new(&mut a.dur).range(0.1..=(max - src_in).max(0.1)).speed(0.05).suffix(" s").max_decimals(2));
    });
    let dur = a.dur;
    row(ui, "Trim in", |ui| {
        ui.add(egui::DragValue::new(&mut a.src_in).range(0.0..=(max - dur).max(0.0)).speed(0.05).suffix(" s").max_decimals(2));
    });
    actions(app, ui);
}

fn project_props(app: &mut KlipApp, ui: &mut egui::Ui) {
    section(ui, "Canvas");
    let w = (ui.available_width() - 8.0) / 2.0;
    for pair in Aspect::ALL.chunks(2) {
        ui.horizontal(|ui| {
            for a in pair {
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 70.0), Sense::click());
                let sel = app.project.aspect == *a;
                let h = anim(ui, ("asp_h", a.label()), resp.hovered());
                let t = anim(ui, ("asp_s", a.label()), sel);
                let p = ui.painter();
                p.rect_filled(r, CornerRadius::same(R_MD), mix(mix(BG0, BG2, h), BG3, t));
                p.rect_stroke(r, CornerRadius::same(R_MD), Stroke::new(1.5, mix(BORDER, ACCENT, t)), StrokeKind::Inside);
                let (cw, ch) = a.canvas();
                let s = 26.0 / cw.max(ch) as f32;
                let fr = egui::Rect::from_center_size(egui::Pos2::new(r.left() + 26.0, r.center().y), Vec2::new(cw as f32 * s, ch as f32 * s));
                p.rect_stroke(fr, CornerRadius::same(2), Stroke::new(1.5, mix(TEXT_DIM, ACCENT, t)), StrokeKind::Inside);
                p.text(egui::Pos2::new(r.left() + 48.0, r.center().y - 8.0), egui::Align2::LEFT_CENTER, a.label(), egui::FontId::proportional(14.0), TEXT);
                p.text(egui::Pos2::new(r.left() + 48.0, r.center().y + 9.0), egui::Align2::LEFT_CENTER, a.hint().split(" ·").next().unwrap_or(""), egui::FontId::proportional(10.5), TEXT_DIM);
                if resp.on_hover_text(a.hint()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.project.aspect = *a;
                }
            }
        });
    }

    ui.add_space(14.0);
    section(ui, "Overview");
    let p = &app.project;
    stat(ui, "Duration", &timecode(p.duration()));
    stat(ui, "Clips", &p.video.len().to_string());
    stat(ui, "Text & captions", &p.texts.len().to_string());
    stat(ui, "Audio tracks", &p.audio.len().to_string());

    ui.add_space(14.0);
    section(ui, "Shortcuts");
    for (k, v) in [
        ("Space", "Play / pause"),
        ("S", "Split at playhead"),
        ("Del", "Delete selected"),
        ("Ctrl D", "Duplicate"),
        ("← →", "Step one frame"),
        ("Ctrl Z", "Undo"),
        ("Ctrl ⇧ Z", "Redo"),
        ("Ctrl wheel", "Zoom timeline"),
        ("Ctrl E", "Export"),
    ] {
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(BG0)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 1))
                .show(ui, |ui| ui.label(RichText::new(k).monospace().size(11.0).color(TEXT)));
            ui.label(RichText::new(v).size(12.0).color(TEXT_DIM));
        });
    }
}

fn stat(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(v).color(TEXT));
        });
    });
}
