//! Centre panel: the canvas (video + text overlays) and transport controls.

use crate::app::{KlipApp, Selection};
use crate::fx::TransitionKind;
use crate::model::{FitMode, TextClip, TextStyle, VideoClip, base_size, timecode};
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
    // Player view: zoom out to reach handles of clips that overflow the
    // canvas, zoom in for detail. Ctrl+wheel zooms at the cursor, dragging
    // empty space pans, double-click empty space fits again.
    let stage_area = Rect::from_min_max(full.min, Pos2::new(full.right(), controls.top()));
    let stage_resp = ui.interact(stage_area, ui.id().with("stage"), Sense::click_and_drag());
    if stage_resp.hovered() {
        let zd = ui.input(|i| i.zoom_delta());
        if zd != 1.0 {
            let old = app.view_zoom;
            let new = (old * zd).clamp(0.2, 4.0);
            if let Some(ptr) = ui.ctx().pointer_hover_pos() {
                let c = stage.center() + app.view_pan;
                let c2 = ptr - (ptr - c) * (new / old);
                app.view_pan = c2 - stage.center();
            }
            app.view_zoom = new;
        }
    }
    let middle = ui.input(|i| i.pointer.middle_down());
    if (stage_resp.dragged() && app.view_zoom != 1.0) || (middle && stage_resp.hovered()) {
        app.view_pan += ui.input(|i| i.pointer.delta());
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if stage_resp.double_clicked() {
        app.view_zoom = 1.0;
        app.view_pan = Vec2::ZERO;
    } else if stage_resp.clicked() && matches!(app.selection, Selection::Video(_) | Selection::Text(_)) {
        app.selection = Selection::None;
    }
    if (app.view_zoom - 1.0).abs() < 1e-3 && !stage_resp.dragged() {
        app.view_pan = Vec2::ZERO;
    }
    let size = size * app.view_zoom;
    // Keep at least part of the canvas on screen.
    let lim = (stage.size() + size) / 2.0 - Vec2::splat(40.0);
    app.view_pan = app.view_pan.clamp(-lim.max(Vec2::ZERO), lim.max(Vec2::ZERO));
    let canvas = Rect::from_center_size(stage.center() + app.view_pan, size);
    let scale = canvas.width() / cw as f32;

    // Clicking empty canvas clears a text selection.
    let bg = ui.interact(canvas, ui.id().with("canvas"), Sense::click());
    if bg.clicked() {
        if let Selection::Text(_) = app.selection {
            app.selection = Selection::None;
        }
    }

    let p = ui.painter_at(stage_area);
    // soft drop shadow
    for i in 1..=4 {
        let e = i as f32 * 3.0;
        p.rect_filled(canvas.expand(e), CornerRadius::same(4 + i as u8 * 2), Color32::from_black_alpha(18));
    }
    if !app.fullscreen {
        p.rect_filled(canvas, CornerRadius::same(4), Color32::BLACK);
    }

    paint_overflow_ghost(app, &ui.painter_at(stage_area), canvas);
    paint_video(app, &p, canvas);

    if app.project.video.is_empty() {
        p.text(canvas.center() - Vec2::new(0.0, 14.0), Align2::CENTER_CENTER, ph::FILM_SLATE, FontId::proportional(36.0), TEXT_FAINT);
        p.text(canvas.center() + Vec2::new(0.0, 22.0), Align2::CENTER_CENTER, "Add a clip to start", FontId::proportional(13.0), TEXT_DIM);
    }

    // Handles live in the stage only — never over the transport bar.
    video_handles(app, ui, canvas, stage_area);
    ui.scope_builder(egui::UiBuilder::new().max_rect(stage_area), |ui| {
        ui.set_clip_rect(stage_area);
        text_overlays(app, ui, canvas, scale);
    });
    controls_bar(app, ui, controls);
}

// ---------------------------------------------------------------------------
// Video layers
// ---------------------------------------------------------------------------

/// Where a clip's frame sits on screen: centre, unrotated size, rotation.
#[derive(Clone, Copy)]
struct Placed {
    center: Pos2,
    size: Vec2,
    rot: f32,
}

impl Placed {
    fn of(clip: &VideoClip, src_w: f32, src_h: f32, vc: Rect) -> Self {
        let (bw, bh) = base_size(clip.fit, src_w, src_h, vc.width(), vc.height());
        let s = clip.scale.clamp(0.05, 4.0);
        Self {
            center: vc.center() + Vec2::new(clip.x * vc.width(), clip.y * vc.height()),
            size: Vec2::new(bw * s, bh * s),
            rot: clip.rotation.to_radians(),
        }
    }

    fn corners(&self) -> [Pos2; 4] {
        let (sin, cos) = self.rot.sin_cos();
        let h = self.size / 2.0;
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sx, sy): (f32, f32)| {
            let (lx, ly) = (sx * h.x, sy * h.y);
            self.center + Vec2::new(lx * cos - ly * sin, lx * sin + ly * cos)
        })
    }

    fn bbox(&self) -> Rect {
        Rect::from_points(&self.corners())
    }

    /// Point at `local` (unrotated offset from centre) in screen space.
    fn at(&self, local: Vec2) -> Pos2 {
        let (sin, cos) = self.rot.sin_cos();
        self.center + Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos)
    }
}

fn paint_quad(p: &egui::Painter, tex: egui::TextureId, q: &Placed, tint: Color32) {
    let c = q.corners();
    let uv = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
    let mut mesh = egui::epaint::Mesh::with_texture(tex);
    for k in 0..4 {
        mesh.vertices.push(egui::epaint::Vertex { pos: c[k], uv: uv[k], color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
}

/// Draw one preview layer into the virtual canvas `vc` at `opacity`.
fn draw_layer(app: &KlipApp, p: &egui::Painter, slot: usize, vc: Rect, opacity: f32) {
    let layer = &app.layers[slot];
    let (Some(clip_id), Some(tex)) = (layer.clip, &layer.tex) else { return };
    let Some(idx) = app.project.video_index(clip_id) else { return };
    let clip = &app.project.video[idx];
    if layer.media != Some(clip.media) {
        return; // frame for this clip hasn't arrived yet
    }
    let Some(m) = app.project.media(clip.media) else { return };
    let start = app.project.video_starts()[idx];
    let fade = clip.fade_alpha(app.playhead - start);
    let a = |v: f32| ((v * opacity).clamp(0.0, 1.0) * 255.0) as u8;

    // Background (part of the clip's frame, so it moves with transitions).
    if clip.fit == FitMode::Blur {
        if let Some(b) = &layer.blur {
            let uv = super::library::cover_uv(m.aspect(), vc.aspect_ratio());
            p.image(b.id(), vc, uv, Color32::from_gray(205).gamma_multiply(opacity));
        }
    } else {
        p.rect_filled(vc, CornerRadius::ZERO, Color32::from_black_alpha(a(1.0)));
    }
    let q = Placed::of(clip, m.width as f32, m.height as f32, vc);
    paint_quad(p, tex.id(), &q, Color32::from_white_alpha(a(1.0)));
    // Clip fade in/out = fade to black.
    if fade < 1.0 {
        p.rect_filled(vc, CornerRadius::ZERO, Color32::from_black_alpha(a(1.0 - fade)));
    }
}

/// Faded preview of the selected clip's parts that fall outside the canvas,
/// so you can see where an enlarged or moved clip's edges are.
fn paint_overflow_ghost(app: &KlipApp, p: &egui::Painter, canvas: Rect) {
    let Selection::Video(id) = app.selection else { return };
    if app.project.transition_at(app.playhead).is_some() {
        return;
    }
    let layer = &app.layers[0];
    let (Some(clip_id), Some(tex)) = (layer.clip, &layer.tex) else { return };
    if clip_id != id {
        return;
    }
    let Some(clip) = app.project.video.iter().find(|c| c.id == id) else { return };
    let Some(m) = app.project.media(clip.media) else { return };
    let q = Placed::of(clip, m.width as f32, m.height as f32, canvas);
    if canvas.contains_rect(q.bbox()) {
        return;
    }
    paint_quad(p, tex.id(), &q, Color32::from_white_alpha(70));
}

fn paint_video(app: &KlipApp, p: &egui::Painter, canvas: Rect) {
    let p = p.with_clip_rect(canvas);
    let Some(tr) = app.project.transition_at(app.playhead) else {
        draw_layer(app, &p, 0, canvas, 1.0);
        return;
    };
    let (i, q) = tr;
    let kind = app.project.video[i].transition.map(|t| t.kind).unwrap_or(TransitionKind::Mix);
    let (w, h) = (canvas.width(), canvas.height());
    let shift = |dx: f32, dy: f32| canvas.translate(Vec2::new(dx, dy));
    let ease = |x: f32| x * x * (3.0 - 2.0 * x);
    match kind {
        TransitionKind::Mix => {
            draw_layer(app, &p, 0, canvas, 1.0);
            draw_layer(app, &p, 1, canvas, q);
        }
        TransitionKind::FadeBlack | TransitionKind::FadeWhite => {
            let col = if kind == TransitionKind::FadeBlack { 0 } else { 255 };
            let (slot, k) = if q < 0.5 { (0, q * 2.0) } else { (1, 2.0 - q * 2.0) };
            draw_layer(app, &p, slot, canvas, 1.0);
            p.rect_filled(canvas, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(col, col, col, (ease(k) * 255.0) as u8));
        }
        TransitionKind::SlideLeft => {
            draw_layer(app, &p, 0, shift(-q * w, 0.0), 1.0);
            draw_layer(app, &p, 1, shift((1.0 - q) * w, 0.0), 1.0);
        }
        TransitionKind::SlideRight => {
            draw_layer(app, &p, 0, shift(q * w, 0.0), 1.0);
            draw_layer(app, &p, 1, shift(-(1.0 - q) * w, 0.0), 1.0);
        }
        TransitionKind::SlideUp => {
            draw_layer(app, &p, 0, shift(0.0, -q * h), 1.0);
            draw_layer(app, &p, 1, shift(0.0, (1.0 - q) * h), 1.0);
        }
        TransitionKind::Wipe => {
            draw_layer(app, &p, 0, canvas, 1.0);
            let r = Rect::from_min_max(Pos2::new(canvas.right() - q * w, canvas.top()), canvas.max);
            draw_layer(app, &p.with_clip_rect(r), 1, canvas, 1.0);
        }
        TransitionKind::ZoomIn => {
            draw_layer(app, &p, 1, canvas, 1.0);
            let z = 1.0 + ease(q) * 0.6;
            draw_layer(app, &p, 0, Rect::from_center_size(canvas.center(), canvas.size() * z), 1.0 - ease(q));
        }
        TransitionKind::Blur => {
            draw_layer(app, &p, 0, canvas, 1.0);
            draw_layer(app, &p, 1, canvas, q);
            let haze = (std::f32::consts::PI * q).sin() * 0.85;
            let slot = if q < 0.5 { 0 } else { 1 };
            if let Some(b) = &app.layers[slot].blur {
                let uv = super::library::cover_uv(b.aspect_ratio(), canvas.aspect_ratio());
                p.image(b.id(), canvas, uv, Color32::from_white_alpha((haze * 255.0) as u8));
            }
        }
    }
}

/// Selection box + move / scale / rotate handles for the clip under the playhead.
fn video_handles(app: &mut KlipApp, ui: &mut egui::Ui, canvas: Rect, area: Rect) {
    if app.project.transition_at(app.playhead).is_some() {
        return; // editing mid-transition would be confusing
    }
    let Some((i, _)) = app.project.video_at(app.playhead) else { return };
    let clip = app.project.video[i].clone();
    let Some(m) = app.project.media(clip.media).cloned() else { return };
    let q = Placed::of(&clip, m.width as f32, m.height as f32, canvas);
    let id = ui.id().with(("vt", clip.id));
    let sel = app.selection == Selection::Video(clip.id);

    let body = ui.interact(q.bbox().intersect(canvas), id.with("body"), Sense::click_and_drag());
    if body.clicked() || body.drag_started() {
        app.selection = Selection::Video(clip.id);
    }
    let p = ui.painter_at(area);
    if !sel && body.hovered() {
        p.add(egui::Shape::closed_line(q.corners().to_vec(), Stroke::new(1.0, Color32::from_white_alpha(70))));
    }
    if body.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let guide = Stroke::new(1.0, GUIDE);
    if body.dragged() && sel {
        let d = body.drag_delta();
        let c = &mut app.project.video[i];
        c.x += d.x / canvas.width();
        c.y += d.y / canvas.height();
        // Magnetic centre lines while dragging.
        if (c.x * canvas.width()).abs() < 6.0 {
            c.x = 0.0;
            p.vline(canvas.center().x, canvas.y_range(), guide);
        }
        if (c.y * canvas.height()).abs() < 6.0 {
            c.y = 0.0;
            p.hline(canvas.x_range(), canvas.center().y, guide);
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if !sel {
        return;
    }

    // Outline.
    p.add(egui::Shape::closed_line(q.corners().to_vec(), Stroke::new(1.5, ACCENT)));

    // Corner handles: uniform scale about the centre.
    for (k, corner) in q.corners().into_iter().enumerate() {
        let r = Rect::from_center_size(corner, Vec2::splat(16.0));
        if !area.contains_rect(r) {
            continue; // off-screen corner: use the inspector instead
        }
        let h = ui.interact(r, id.with(("corner", k)), Sense::drag());
        let hot = h.hovered() || h.dragged();
        p.circle(corner, if hot { 6.0 } else { 5.0 }, Color32::WHITE, Stroke::new(1.5, ACCENT));
        if hot {
            let diag = if k % 2 == 0 { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw };
            ui.ctx().set_cursor_icon(diag);
        }
        if h.drag_started() {
            let d0 = h.interact_pointer_pos().map_or(1.0, |pp| (pp - q.center).length()).max(1.0);
            ui.ctx().data_mut(|dm| dm.insert_temp(id.with("scale0"), (clip.scale, d0)));
        }
        if h.dragged() {
            if let (Some(pp), Some((s0, d0))) = (h.interact_pointer_pos(), ui.ctx().data(|dm| dm.get_temp::<(f32, f32)>(id.with("scale0")))) {
                let s = (s0 * (pp - q.center).length() / d0).clamp(0.05, 4.0);
                // Gentle snap to 100%.
                app.project.video[i].scale = if (s - 1.0).abs() < 0.03 { 1.0 } else { s };
            }
        }
    }

    // Rotate handle under the box.
    // Rotate handle below the box, or above it if there's no room.
    let mut edge = q.size.y / 2.0;
    let mut below = q.at(Vec2::new(0.0, edge + 26.0));
    if !area.shrink(14.0).contains(below) {
        edge = -q.size.y / 2.0;
        below = q.at(Vec2::new(0.0, edge - 26.0));
    }
    if !area.shrink(14.0).contains(below) {
        return;
    }
    let rr = Rect::from_center_size(below, Vec2::splat(26.0));
    let rh = ui.interact(rr, id.with("rotate"), Sense::drag());
    let hot = rh.hovered() || rh.dragged();
    p.line_segment([q.at(Vec2::new(0.0, edge)), below], Stroke::new(1.0, ACCENT.gamma_multiply(0.7)));
    p.circle(below, if hot { 12.0 } else { 11.0 }, BG2, Stroke::new(1.5, ACCENT));
    p.text(below, Align2::CENTER_CENTER, ph::ARROW_CLOCKWISE, FontId::proportional(13.0), TEXT);
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if rh.drag_started() {
        let a0 = rh.interact_pointer_pos().map_or(0.0, |pp| (pp - q.center).angle());
        ui.ctx().data_mut(|dm| dm.insert_temp(id.with("rot0"), (clip.rotation, a0)));
    }
    if rh.dragged() {
        if let (Some(pp), Some((r0, a0))) = (rh.interact_pointer_pos(), ui.ctx().data(|dm| dm.get_temp::<(f32, f32)>(id.with("rot0")))) {
            let mut deg = r0 + ((pp - q.center).angle() - a0).to_degrees();
            deg = (deg + 180.0).rem_euclid(360.0) - 180.0;
            let snap = (deg / 90.0).round() * 90.0;
            if (deg - snap).abs() < 4.0 {
                deg = snap + 0.0; // normalise -0
            }
            app.project.video[i].rotation = deg;
            let label = p.layout_no_wrap(format!("{}°", deg.round() as i32), FontId::proportional(12.0), Color32::WHITE);
            let lr = Rect::from_center_size(below + Vec2::new(0.0, 26.0), label.size() + Vec2::new(12.0, 6.0));
            p.rect_filled(lr, CornerRadius::same(R_SM), Color32::from_black_alpha(200));
            p.galley(lr.min + Vec2::new(6.0, 3.0), label, Color32::WHITE);
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
    // Clip to the visible stage too (the canvas may be larger when zoomed in).
    let stage_clip = ui.clip_rect();
    let p = ui.painter_at(canvas.intersect(stage_clip));
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
    // View zoom: − [Fit / 50%] +
    if medium {
        let zx = r.right() - 74.0;
        if btn(ui, Pos2::new(zx - 70.0, mid.y), ph::MINUS, false, "vz_out", "Zoom view out (Ctrl+wheel)") {
            app.view_zoom = (app.view_zoom / 1.25).max(0.2);
        }
        if btn(ui, Pos2::new(zx, mid.y), ph::PLUS, false, "vz_in", "Zoom view in (Ctrl+wheel)") {
            app.view_zoom = (app.view_zoom * 1.25).min(4.0);
        }
        let label = if (app.view_zoom - 1.0).abs() < 1e-3 { "Fit".to_string() } else { format!("{:.0}%", app.view_zoom * 100.0) };
        let lr = Rect::from_center_size(Pos2::new(zx - 35.0, mid.y), Vec2::new(46.0, 26.0));
        let lresp = ui.interact(lr, ui.id().with("vz_fit"), Sense::click()).on_hover_text("Fit to window");
        let h = anim(ui, "vz_fit_h", lresp.hovered());
        if h > 0.01 {
            ui.painter().rect_filled(lr, CornerRadius::same(R_SM), BG3.gamma_multiply(h));
        }
        ui.painter().text(lr.center(), Align2::CENTER_CENTER, label, FontId::new(12.5, medium_family()), mix(TEXT_DIM, TEXT, h));
        if lresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.view_zoom = 1.0;
            app.view_pan = Vec2::ZERO;
        }
    }
    // Canvas label.
    if wide && !app.fullscreen && r.width() > 760.0 {
        let (cw, ch) = app.project.aspect.canvas();
        ui.painter().text(
            Pos2::new(r.right() - 186.0, mid.y),
            Align2::RIGHT_CENTER,
            format!("{} · {}×{}", app.project.aspect.label(), cw, ch),
            FontId::proportional(11.5),
            TEXT_FAINT,
        );
    }
}

