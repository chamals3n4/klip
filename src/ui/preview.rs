//! Centre panel: the canvas (video + text overlays) and transport controls.

use crate::app::{KlipApp, Selection};
use crate::model::{FitMode, TextClip, TextStyle, timecode};
use crate::theme::*;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;

pub fn preview(app: &mut KlipApp, ui: &mut egui::Ui) {
    let full = ui.available_rect_before_wrap();
    let controls_h = if app.fullscreen { 64.0 } else { 54.0 };
    let m = if app.fullscreen { 24.0 } else { 16.0 };
    let stage = Rect::from_min_max(full.min + Vec2::new(m, m), Pos2::new(full.right() - m, full.bottom() - controls_h));
    let controls = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - controls_h), full.max);

    let (cw, ch) = app.project.aspect.canvas();
    let canvas_aspect = cw as f32 / ch as f32;
    let mut size = Vec2::new(stage.height() * canvas_aspect, stage.height());
    if size.x > stage.width() {
        size = Vec2::new(stage.width(), stage.width() / canvas_aspect);
    }
    let canvas = Rect::from_center_size(stage.center(), size);
    let scale = canvas.width() / cw as f32;

    // Clicking empty canvas clears a text selection.
    let bg = ui.interact(canvas, ui.id().with("canvas"), Sense::click());
    if bg.clicked() {
        if let Selection::Text(_) = app.selection {
            app.selection = Selection::None;
        }
    }

    let p = ui.painter_at(full);
    // soft drop shadow
    for i in 1..=4 {
        let e = i as f32 * 3.0;
        p.rect_filled(canvas.expand(e), CornerRadius::same(4 + i as u8 * 2), Color32::from_black_alpha(18));
    }
    if !app.fullscreen {
        p.rect_filled(canvas, CornerRadius::same(4), Color32::BLACK);
    }

    paint_video(app, &p, canvas);

    if app.project.video.is_empty() {
        p.text(canvas.center() - Vec2::new(0.0, 14.0), Align2::CENTER_CENTER, ph::FILM_SLATE, FontId::proportional(36.0), TEXT_FAINT);
        p.text(canvas.center() + Vec2::new(0.0, 22.0), Align2::CENTER_CENTER, "Add a clip to start", FontId::proportional(13.0), TEXT_DIM);
    }

    text_overlays(app, ui, canvas, scale);
    controls_bar(app, ui, controls);
}

fn paint_video(app: &KlipApp, p: &egui::Painter, canvas: Rect) {
    let Some(tex) = &app.preview_tex else { return };
    if app.preview_media.is_none() {
        return;
    }
    let Some((i, _)) = app.project.video_at(app.playhead) else { return };
    let clip = &app.project.video[i];
    let [tw, th] = tex.size();
    let src_aspect = tw as f32 / th.max(1) as f32;
    let full_uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    let p = p.with_clip_rect(canvas);
    let contain = |aspect: f32| {
        let ca = canvas.aspect_ratio();
        if aspect > ca {
            Rect::from_center_size(canvas.center(), Vec2::new(canvas.width(), canvas.width() / aspect))
        } else {
            Rect::from_center_size(canvas.center(), Vec2::new(canvas.height() * aspect, canvas.height()))
        }
    };
    match clip.fit {
        FitMode::Fill => {
            p.image(tex.id(), canvas, super::library::cover_uv(src_aspect, canvas.aspect_ratio()), Color32::WHITE);
        }
        FitMode::Fit => {
            p.image(tex.id(), contain(src_aspect), full_uv, Color32::WHITE);
        }
        FitMode::Blur => {
            if let Some(b) = &app.preview_blur {
                p.image(b.id(), canvas, super::library::cover_uv(src_aspect, canvas.aspect_ratio()), Color32::from_gray(205));
            }
            p.image(tex.id(), contain(src_aspect), full_uv, Color32::WHITE);
        }
    }
}

fn text_overlays(app: &mut KlipApp, ui: &mut egui::Ui, canvas: Rect, scale: f32) {
    let t = app.playhead;
    let visible: Vec<usize> = app
        .project
        .texts
        .iter()
        .enumerate()
        .filter(|(_, c)| t >= c.start && t < c.end() || app.selection == Selection::Text(c.id) && t >= c.start - 1e-6 && t <= c.end())
        .map(|(i, _)| i)
        .collect();
    let p = ui.painter_at(canvas);
    for i in visible {
        let clip = app.project.texts[i].clone();
        let center = Pos2::new(canvas.left() + clip.x * canvas.width(), canvas.top() + clip.y * canvas.height());
        let bounds = paint_text(&p, &clip, center, scale);
        let resp = ui.interact(bounds.expand(6.0), ui.id().with(("text", clip.id)), Sense::click_and_drag());
        let sel = app.selection == Selection::Text(clip.id);
        if resp.hovered() && !sel {
            p.rect_stroke(bounds.expand(6.0), CornerRadius::same(4), Stroke::new(1.0, Color32::from_white_alpha(90)), StrokeKind::Outside);
        }
        if sel {
            let r = bounds.expand(6.0);
            p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, ACCENT), StrokeKind::Outside);
            for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                p.circle(c, 4.0, Color32::WHITE, Stroke::new(1.5, ACCENT));
            }
        }
        if resp.clicked() || resp.drag_started() {
            app.selection = Selection::Text(clip.id);
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            let c = &mut app.project.texts[i];
            c.x = (c.x + d.x / canvas.width()).clamp(0.0, 1.0);
            c.y = (c.y + d.y / canvas.height()).clamp(0.0, 1.0);
            // Snap to centre lines.
            let cx = canvas.left() + c.x * canvas.width();
            let cy = canvas.top() + c.y * canvas.height();
            let guide = Stroke::new(1.0, GUIDE);
            if (cx - canvas.center().x).abs() < 5.0 {
                c.x = 0.5;
                p.vline(canvas.center().x, canvas.y_range(), guide);
            }
            if (cy - canvas.center().y).abs() < 5.0 {
                c.y = 0.5;
                p.hline(canvas.x_range(), canvas.center().y, guide);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
}

/// Draw a text clip centred at `center`, `scale` = screen px per canvas px.
/// Returns the text bounds. Mirrors the drawtext styling used on export.
pub fn paint_text(p: &egui::Painter, clip: &TextClip, center: Pos2, scale: f32) -> Rect {
    let size = (clip.size * scale).max(1.0);
    let color = Color32::from_rgb(clip.color[0], clip.color[1], clip.color[2]);
    let mut job = egui::text::LayoutJob::simple(clip.text.clone(), FontId::new(size, text_family()), color, f32::INFINITY);
    job.halign = egui::Align::Center;
    let galley = p.layout_job(job);
    let pos = center - galley.rect.center().to_vec2();
    let bounds = galley.rect.translate(pos.to_vec2());
    match clip.style {
        TextStyle::Plain => {}
        TextStyle::Outline => {
            let w = (size * 0.07).max(0.8);
            let n = 12;
            for k in 0..n {
                let a = std::f32::consts::TAU * k as f32 / n as f32;
                p.galley_with_override_text_color(pos + Vec2::new(a.cos(), a.sin()) * w, galley.clone(), Color32::BLACK);
            }
        }
        TextStyle::Boxed => {
            p.rect_filled(bounds.expand(size * 0.28), CornerRadius::same((size * 0.12).min(255.0) as u8), Color32::from_black_alpha(158));
        }
        TextStyle::Shadow => {
            let d = (size * 0.06).max(0.8);
            p.galley_with_override_text_color(pos + Vec2::splat(d), galley.clone(), Color32::from_black_alpha(178));
        }
    }
    p.galley(pos, galley, color);
    bounds
}

fn controls_bar(app: &mut KlipApp, ui: &mut egui::Ui, r: Rect) {
    let total = app.project.duration();
    let mid = r.center();
    let wide = r.width() >= 600.0;
    let medium = r.width() >= 420.0;

    // Timecode (left): current in white, total dimmed.
    let mut job = egui::text::LayoutJob::default();
    let mono = FontId::monospace(13.0);
    job.append(&timecode(app.playhead), 0.0, egui::TextFormat::simple(mono.clone(), TEXT));
    if medium {
        job.append(&format!(" / {}", timecode(total)), 0.0, egui::TextFormat::simple(mono, TEXT_FAINT));
    }
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(r.left() + 18.0, mid.y - galley.size().y / 2.0), galley, TEXT);

    // Transport (centre).
    let btn = |ui: &mut egui::Ui, c: Pos2, icon: &str, big: bool, id: &str, tip: &str| -> bool {
        let rad = if big { 20.0 } else { 16.0 };
        let rr = Rect::from_center_size(c, Vec2::splat(rad * 2.0));
        let resp = ui.interact(rr, ui.id().with(id), Sense::click()).on_hover_text(tip);
        let h = anim(ui, ("transport", id), resp.hovered());
        let press = anim(ui, ("transport_p", id), resp.is_pointer_button_down_on());
        let p = ui.painter();
        if big {
            let rad = rad - press * 1.2;
            p.circle_filled(c, rad, mix(ACCENT, ACCENT_HI, h * 0.6));
            if icon == ph::PLAY {
                paint_play(p, c, 13.0, ON_ACCENT);
            } else {
                paint_pause(p, c, 13.0, ON_ACCENT);
            }
        } else {
            if h > 0.01 {
                p.rect_filled(rr.shrink(1.0), CornerRadius::same(R_SM + 1), BG3.gamma_multiply(h));
            }
            p.text(c, Align2::CENTER_CENTER, icon, FontId::proportional(18.0), mix(TEXT_DIM, TEXT, h));
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    };
    // Keep the transport clear of the timecode on narrow layouts.
    let cx = if medium { mid.x } else { mid.x.max(r.left() + 120.0 + 60.0) };
    let c = Pos2::new(cx, mid.y);
    if wide {
        if btn(ui, c - Vec2::new(84.0, 0.0), ph::CARET_LEFT, false, "prevf", "Previous frame (←)") {
            app.seek(app.playhead - 1.0 / 30.0);
        }
        if btn(ui, c + Vec2::new(84.0, 0.0), ph::CARET_RIGHT, false, "nextf", "Next frame (→)") {
            app.seek(app.playhead + 1.0 / 30.0);
        }
    }
    if btn(ui, c - Vec2::new(46.0, 0.0), ph::SKIP_BACK, false, "start", "Go to start (Home)") {
        app.seek(0.0);
    }
    let icon = if app.playing { ph::PAUSE } else { ph::PLAY };
    if btn(ui, c, icon, true, "play", "Play / pause (Space)") {
        app.toggle_play();
    }
    if btn(ui, c + Vec2::new(46.0, 0.0), ph::SKIP_FORWARD, false, "end", "Go to end (End)") {
        app.seek(total);
    }

    // Fullscreen toggle (right).
    let (icon, tip) = if app.fullscreen { (ph::CORNERS_IN, "Exit full screen (Esc)") } else { (ph::CORNERS_OUT, "Full screen (F)") };
    if btn(ui, Pos2::new(r.right() - 34.0, mid.y), icon, false, "fullscreen", tip) {
        app.set_fullscreen(!app.fullscreen);
    }
    // Canvas label.
    if wide && !app.fullscreen {
        let (cw, ch) = app.project.aspect.canvas();
        ui.painter().text(
            Pos2::new(r.right() - 62.0, mid.y),
            Align2::RIGHT_CENTER,
            format!("{} · {}×{}", app.project.aspect.label(), cw, ch),
            FontId::proportional(11.5),
            TEXT_FAINT,
        );
    }
}

