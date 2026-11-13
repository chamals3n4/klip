//! The timeline: ruler, text lanes, magnetic main track, audio lanes.
//!
//! Interaction model (CapCut-like):
//! - click empty space / ruler: move playhead; drag: scrub
//! - click a clip: select; drag its body: move (text/audio) or reorder (main)
//! - drag a clip edge: trim; edges snap to playhead and other clip edges
//! - wheel: scroll, Ctrl+wheel: zoom around the cursor

use crate::app::{AudioDrag, ClipKind, DragMode, KlipApp, LeftTab, MediaDrag, Selection, TlDrag};
use crate::fx::TransitionKind;
use crate::media::WAVE_RATE;
use crate::model::{FPS, Id, MediaKind};
use crate::theme::*;
use crate::ui::MAX_PPS;
use crate::ui::library::{elide, round_mask};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;
use std::collections::HashMap;

const TOOLBAR_H: f32 = 44.0;
const HEADER_W: f32 = 54.0;
const RULER_H: f32 = 26.0;
const TEXT_H: f32 = 28.0;
const MAIN_H: f32 = 64.0;
const AUDIO_H: f32 = 40.0;
const GAP: f32 = 8.0;
const ROW_BG: Color32 = Color32::from_rgb(0x17, 0x18, 0x1e);

struct Geo {
    content: Rect,
    pps: f64,
    scroll: f64,
}

impl Geo {
    fn x(&self, t: f64) -> f32 {
        self.content.left() + ((t - self.scroll) * self.pps) as f32
    }
    fn t(&self, x: f32) -> f64 {
        self.scroll + (x - self.content.left()) as f64 / self.pps
    }
}

/// Greedy lane packing so overlapping clips stack instead of hiding each other.
fn lanes(items: &[(Id, f64, f64)]) -> (HashMap<Id, usize>, usize) {
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut ends: Vec<f64> = vec![];
    let mut map = HashMap::new();
    for (id, s, e) in sorted {
        let lane = match ends.iter().position(|&end| end <= s + 1e-6) {
            Some(l) => l,
            None => {
                ends.push(0.0);
                ends.len() - 1
            }
        };
        ends[lane] = e;
        map.insert(id, lane);
    }
    let n = ends.len().max(1);
    (map, n)
}

pub fn timeline(app: &mut KlipApp, ui: &mut egui::Ui) {
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    // Zoom limits: fully out = whole project (+ a little room) fits on screen.
    let project_end = app
        .project
        .texts
        .iter()
        .map(|t| t.end())
        .chain(app.project.audio.iter().map(|a| a.end()))
        .fold(app.project.duration(), f64::max)
        .max(5.0);
    let content_w = (full.width() - HEADER_W - 10.0).max(100.0);
    let fit_pps = (content_w / (project_end as f32 * 1.05)).min(MAX_PPS);
    app.tl.min_pps = fit_pps.min(4.0).max(0.005);
    app.tl.pps = app.tl.pps.clamp(app.tl.min_pps, MAX_PPS);
    let toolbar = Rect::from_min_size(full.min, Vec2::new(full.width(), TOOLBAR_H));
    toolbar_ui(app, ui, toolbar);

    let body = Rect::from_min_max(Pos2::new(full.left(), toolbar.bottom()), full.max);
    let content = Rect::from_min_max(Pos2::new(body.left() + HEADER_W, body.top()), Pos2::new(body.right() - 10.0, body.bottom() - 6.0));
    let ctx = ui.ctx().clone();
    let pointer = ctx.pointer_hover_pos();
    let total = app.project.duration();

    // ---------------------------------------------------------- scroll / zoom
    let hovering = pointer.is_some_and(|p| body.contains(p));
    if hovering {
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if zoom != 1.0 {
            let px = pointer.unwrap().x;
            let g = Geo { content, pps: app.tl.pps as f64, scroll: app.tl.scroll };
            let anchor = g.t(px);
            app.tl.pps = (app.tl.pps * zoom).clamp(app.tl.min_pps, MAX_PPS);
            app.tl.scroll = anchor - (px - content.left()) as f64 / app.tl.pps as f64;
        } else if scroll != Vec2::ZERO {
            app.tl.scroll -= (scroll.x + scroll.y) as f64 / app.tl.pps as f64;
        }
    }
    let visible_secs = content.width() as f64 / app.tl.pps as f64;
    let max_end = app
        .project
        .texts
        .iter()
        .map(|t| t.end())
        .chain(app.project.audio.iter().map(|a| a.end()))
        .fold(total, f64::max);
    app.tl.scroll = app.tl.scroll.clamp(0.0, (max_end - visible_secs * 0.5).max(0.0));
    // Follow the playhead while playing.
    if app.playing {
        if app.playhead > app.tl.scroll + visible_secs * 0.92 || app.playhead < app.tl.scroll {
            app.tl.scroll = (app.playhead - visible_secs * 0.08).max(0.0);
        }
    }

    let g = Geo { content, pps: app.tl.pps as f64, scroll: app.tl.scroll };

    // ---------------------------------------------------------------- layout
    let text_items: Vec<_> = app.project.texts.iter().map(|t| (t.id, t.start, t.end())).collect();
    let (text_lane, n_text) = lanes(&text_items);
    let audio_items: Vec<_> = app.project.audio.iter().map(|a| (a.id, a.start, a.end())).collect();
    let (audio_lane, n_audio) = lanes(&audio_items);

    let ruler = Rect::from_min_max(Pos2::new(content.left(), body.top()), Pos2::new(content.right(), body.top() + RULER_H));
    let mut y = ruler.bottom() + 10.0;
    let text_row = Rect::from_min_max(Pos2::new(content.left(), y), Pos2::new(content.right(), y + TEXT_H * n_text as f32 + 2.0 * (n_text as f32 - 1.0)));
    y = text_row.bottom() + GAP;
    let main_row = Rect::from_min_max(Pos2::new(content.left(), y), Pos2::new(content.right(), y + MAIN_H));
    y = main_row.bottom() + GAP;
    let audio_row = Rect::from_min_max(Pos2::new(content.left(), y), Pos2::new(content.right(), y + AUDIO_H * n_audio as f32 + 2.0 * (n_audio as f32 - 1.0)));
    let text_lane_rect = |lane: usize| {
        let top = text_row.top() + lane as f32 * (TEXT_H + 2.0);
        Rect::from_min_max(Pos2::new(content.left(), top), Pos2::new(content.right(), top + TEXT_H))
    };
    let audio_lane_rect = |lane: usize| {
        let top = audio_row.top() + lane as f32 * (AUDIO_H + 2.0);
        Rect::from_min_max(Pos2::new(content.left(), top), Pos2::new(content.right(), top + AUDIO_H))
    };

    let painter = ui.painter_at(body);
    let clip_painter = ui.painter_at(content.expand2(Vec2::new(0.0, 4.0)));

    // ------------------------------------------------------------ background
    let bg = ui.interact(content, ui.id().with("tl_bg"), Sense::click_and_drag());
    let ruler_resp = ui.interact(ruler, ui.id().with("tl_ruler"), Sense::click_and_drag());
    for r in [&bg, &ruler_resp] {
        if r.drag_started() || r.clicked() || r.is_pointer_button_down_on() {
            if let Some(p) = r.interact_pointer_pos() {
                app.seek(g.t(p.x));
                if r.drag_started() {
                    app.tl.drag = TlDrag::Scrub;
                }
            }
        }
    }
    if bg.clicked() {
        app.selection = Selection::None;
    }

    // Row backgrounds and headers.
    for (row, icon, tip) in [
        (text_row, ph::TEXT_T, "Text & captions"),
        (main_row, ph::FILM_STRIP, "Main track"),
        (audio_row, ph::MUSIC_NOTES, "Music & sound"),
    ] {
        painter.rect_filled(Rect::from_min_max(Pos2::new(content.left(), row.top()), Pos2::new(content.right(), row.bottom())), CornerRadius::same(6), ROW_BG);
        let hc = Pos2::new(body.left() + HEADER_W / 2.0, row.center().y);
        let hr = Rect::from_center_size(hc, Vec2::new(32.0, 28.0));
        painter.rect_filled(hr, CornerRadius::same(6), BG2);
        painter.text(hc, Align2::CENTER_CENTER, icon, FontId::proportional(15.0), TEXT_DIM);
        ui.interact(hr, ui.id().with(("hdr", tip)), Sense::hover()).on_hover_text(tip);
    }

    paint_ruler(&painter, &g, ruler);

    // ------------------------------------------------------------- main track
    let starts = app.project.video_starts();
    let dragging_id = match app.tl.drag {
        TlDrag::Reorder { id, .. } => Some(id),
        _ => None,
    };
    if app.project.video.is_empty() {
        let r = Rect::from_min_size(main_row.min + Vec2::new(4.0, 4.0), Vec2::new((content.width() * 0.6).min(460.0), MAIN_H - 8.0));
        dashed_rrect(&painter, r, 6.0, Stroke::new(1.2, BG4));
        painter.text(r.center(), Align2::CENTER_CENTER, format!("{}  Drag photos & videos here", ph::PLUS), FontId::proportional(13.0), TEXT_DIM);
    }
    let mut clip_rects: Vec<(Id, Rect)> = vec![];
    for (i, c) in app.project.video.iter().enumerate() {
        let r = Rect::from_min_max(Pos2::new(g.x(starts[i]) + 1.0, main_row.top() + 2.0), Pos2::new(g.x(starts[i] + c.dur()) - 1.0, main_row.bottom() - 2.0));
        clip_rects.push((c.id, r));
        if r.right() < content.left() || r.left() > content.right() {
            continue;
        }
        let sel = app.selection == Selection::Video(c.id);
        let alpha = if dragging_id == Some(c.id) { 0.35 } else { 1.0 };
        paint_video_clip(app, &clip_painter, r, c.id, sel, alpha, &g);
    }

    // ------------------------------------------------------------ text track
    let mut text_rects = vec![];
    for t in &app.project.texts {
        let lane = text_lane.get(&t.id).copied().unwrap_or(0);
        let row = text_lane_rect(lane);
        let r = Rect::from_min_max(Pos2::new(g.x(t.start) + 1.0, row.top()), Pos2::new(g.x(t.end()) - 1.0, row.bottom()));
        text_rects.push((t.id, r));
        if r.right() < content.left() || r.left() > content.right() {
            continue;
        }
        let sel = app.selection == Selection::Text(t.id);
        let base = if t.is_caption { CAPTION_CLIP } else { TEXT_CLIP };
        clip_painter.rect_filled(r, CornerRadius::same(5), if sel { base } else { base.gamma_multiply(0.75) });
        let label = t.text.lines().next().unwrap_or("").to_string();
        let lp = clip_painter.with_clip_rect(r.shrink(2.0).intersect(content));
        lp.text(
            Pos2::new(r.left().max(content.left()) + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            format!("{}  {}", if t.is_caption { ph::SUBTITLES } else { ph::TEXT_T }, elide(&label, r.width() - 30.0, 6.5)),
            FontId::proportional(11.5),
            Color32::WHITE,
        );
        if sel {
            selection_frame(&clip_painter, r);
        }
    }

    // ----------------------------------------------------------- audio track
    let mut audio_rects = vec![];
    if app.project.audio.is_empty() {
        painter.text(
            Pos2::new(content.left() + 12.0, audio_row.center().y),
            Align2::LEFT_CENTER,
            "Add music from the Audio tab",
            FontId::proportional(11.5),
            TEXT_FAINT,
        );
    }
    if app.project.texts.is_empty() {
        painter.text(
            Pos2::new(content.left() + 12.0, text_row.center().y),
            Align2::LEFT_CENTER,
            "Text & captions appear here",
            FontId::proportional(11.5),
            TEXT_FAINT,
        );
    }
    for a in &app.project.audio {
        let lane = audio_lane.get(&a.id).copied().unwrap_or(0);
        let row = audio_lane_rect(lane);
        let r = Rect::from_min_max(Pos2::new(g.x(a.start) + 1.0, row.top()), Pos2::new(g.x(a.end()) - 1.0, row.bottom()));
        audio_rects.push((a.id, r));
        if r.right() < content.left() || r.left() > content.right() {
            continue;
        }
        let sel = app.selection == Selection::Audio(a.id);
        clip_painter.rect_filled(r, CornerRadius::same(5), if sel { AUDIO_CLIP.gamma_multiply(0.8) } else { AUDIO_CLIP.gamma_multiply(0.5) });
        let wp = clip_painter.with_clip_rect(r.shrink(1.0).intersect(content));
        if let Some(w) = app.assets.get(&a.media).and_then(|x| x.wave.clone()) {
            let base = r.bottom() - 4.0;
            let maxh = r.height() - 18.0;
            let x0 = r.left().max(content.left());
            let x1 = r.right().min(content.right());
            let fades = a.fades();
            let mut x = x0;
            while x < x1 {
                let local = (x - r.left()) as f64 / g.pps;
                let ts = a.src_in + local;
                let v = w.get((ts * WAVE_RATE) as usize).copied().unwrap_or(0.0) * a.volume.min(1.5) / 1.5_f32.max(1.0) * fades.gain(local);
                let h = (v * maxh).max(1.0);
                wp.rect_filled(Rect::from_min_max(Pos2::new(x, base - h), Pos2::new(x + 1.6, base)), CornerRadius::ZERO, Color32::from_rgb(0x6f, 0xf0, 0xb8).gamma_multiply(0.75));
                x += 3.0;
            }
        }
        fade_curve(&wp, r, a.fades(), &g);
        let name = app.project.media(a.media).map(|m| m.name.clone()).unwrap_or_default();
        wp.text(Pos2::new(r.left().max(content.left()) + 8.0, r.top() + 8.0), Align2::LEFT_CENTER, format!("{} {}", ph::MUSIC_NOTE, elide(&name, r.width() - 30.0, 6.5)), FontId::proportional(10.5), Color32::WHITE);
        if sel {
            selection_frame(&clip_painter, r);
        }
    }

    // ----------------------------------------------------- clip interactions
    let mut hits: Vec<(ClipKind, Id, Rect)> = vec![];
    hits.extend(clip_rects.iter().map(|(id, r)| (ClipKind::Video, *id, *r)));
    hits.extend(text_rects.iter().map(|(id, r)| (ClipKind::Text, *id, *r)));
    hits.extend(audio_rects.iter().map(|(id, r)| (ClipKind::Audio, *id, *r)));
    let ptr_t = pointer.map(|p| g.t(p.x));
    for (kind, id, r) in &hits {
        let vis = r.intersect(content);
        if vis.width() <= 0.0 {
            continue;
        }
        let resp = ui.interact(vis, ui.id().with(("clip", *id)), Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            app.selection = match kind {
                ClipKind::Video => Selection::Video(*id),
                ClipKind::Text => Selection::Text(*id),
                ClipKind::Audio => Selection::Audio(*id),
            };
        }
        if resp.hovered() && matches!(app.tl.drag, TlDrag::None) {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                if *kind == ClipKind::Video {
                    app.tl.drag = TlDrag::Reorder { id: *id, grab_dx: p.x - r.left() };
                } else {
                    app.tl.drag = begin_drag(app, *kind, *id, DragMode::Move, g.t(p.x));
                }
            }
        }
        // Trim handles (registered after the body so they win the hit-test).
        let hw = if r.width() > 30.0 { 8.0 } else { 4.0 };
        for (mode, hr) in [
            (DragMode::TrimL, Rect::from_min_max(Pos2::new(r.left() - 3.0, r.top()), Pos2::new(r.left() + hw, r.bottom()))),
            (DragMode::TrimR, Rect::from_min_max(Pos2::new(r.right() - hw, r.top()), Pos2::new(r.right() + 3.0, r.bottom()))),
        ] {
            if !content.intersects(hr) {
                continue;
            }
            let h = ui.interact(hr, ui.id().with(("trim", *id, mode == DragMode::TrimL)), Sense::drag());
            if h.hovered() || h.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if h.drag_started() {
                app.selection = match kind {
                    ClipKind::Video => Selection::Video(*id),
                    ClipKind::Text => Selection::Text(*id),
                    ClipKind::Audio => Selection::Audio(*id),
                };
                if let Some(t) = ptr_t {
                    app.tl.drag = begin_drag(app, *kind, *id, mode, t);
                }
            }
        }
    }

    // ------------------------------------------- transitions / cut markers
    // A transition shows as a badge over the overlap; a hard cut reveals a
    // small "add transition" button when hovered.
    let starts_now = app.project.video_starts();
    let joints: Vec<(usize, Id, f64, f64)> = (0..app.project.video.len().saturating_sub(1))
        .map(|k| {
            let d = app.project.trans_dur(k);
            let mid = if d > 0.0 { starts_now[k + 1] + d / 2.0 } else { starts_now[k + 1] };
            (k, app.project.video[k].id, d, mid)
        })
        .collect();
    let dragging_anything = !matches!(app.tl.drag, TlDrag::None);
    for (_k, id, d, mid) in joints {
        let cx = g.x(mid);
        if cx < content.left() - 20.0 || cx > content.right() + 20.0 {
            continue;
        }
        let jid = ui.id().with(("joint", id));
        if d > 0.0 {
            let w = ((d * g.pps) as f32).max(28.0);
            let r = Rect::from_center_size(Pos2::new(cx, main_row.center().y + 8.0), Vec2::new(w, 24.0));
            let resp = ui.interact(r, jid, Sense::click());
            let sel = app.selection == Selection::Transition(id);
            let h = anim(ui, ("joint_h", id), resp.hovered());
            let s = anim(ui, ("joint_s", id), sel);
            let fill = mix(mix(Color32::from_black_alpha(185), Color32::from_black_alpha(225), h), ACCENT, s);
            clip_painter.rect_filled(r, CornerRadius::same(R_SM), fill);
            clip_painter.rect_stroke(r, CornerRadius::same(R_SM), Stroke::new(1.0, mix(Color32::from_white_alpha(90), Color32::WHITE, s.max(h))), StrokeKind::Inside);
            clip_painter.text(r.center(), Align2::CENTER_CENTER, ph::ARROWS_LEFT_RIGHT, FontId::proportional(14.0), Color32::WHITE);
            if resp.on_hover_text("Transition").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.selection = Selection::Transition(id);
            }
        } else {
            let r = Rect::from_center_size(Pos2::new(cx, main_row.center().y + 8.0), Vec2::splat(24.0));
            let near = !dragging_anything && pointer.is_some_and(|p| main_row.contains(p) && (p.x - cx).abs() < 16.0);
            let resp = ui.interact(r, jid, Sense::click());
            let vis = anim(ui, ("joint_v", id), near || resp.hovered());
            if vis > 0.01 {
                let h = anim(ui, ("joint_h", id), resp.hovered());
                clip_painter.rect_filled(r, CornerRadius::same(R_SM), mix(BG1, ACCENT, h).gamma_multiply(vis));
                clip_painter.rect_stroke(r, CornerRadius::same(R_SM), Stroke::new(1.0, Color32::from_white_alpha((160.0 * vis) as u8)), StrokeKind::Inside);
                clip_painter.text(r.center(), Align2::CENTER_CENTER, ph::PLUS, FontId::proportional(13.0), Color32::from_white_alpha((255.0 * vis) as u8));
            }
            if vis > 0.5 && resp.on_hover_text("Add transition").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.selection = Selection::Video(id);
                app.apply_transition(TransitionKind::Mix);
                app.left_tab = LeftTab::Transitions;
            }
        }
    }

    // --------------------------------------------------------- active drag
    let down = ctx.input(|i| i.pointer.primary_down());
    app.tl.snap_line = None;
    match app.tl.drag {
        TlDrag::None => {}
        TlDrag::Scrub => {
            if down {
                if let Some(p) = pointer {
                    app.seek(g.t(p.x));
                    edge_autoscroll(app, &g, p.x);
                }
            } else {
                app.tl.drag = TlDrag::None;
            }
        }
        TlDrag::Clip { .. } => {
            if down {
                if let Some(t) = ptr_t {
                    apply_drag(app, t, &g);
                    edge_autoscroll(app, &g, pointer.unwrap().x);
                }
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else {
                app.tl.drag = TlDrag::None;
                app.invalidate_preview();
            }
        }
        TlDrag::Reorder { id, grab_dx } => {
            let from = app.project.video_index(id);
            let target = pointer.map(|p| insertion_index(&clip_rects, Some(id), p.x));
            if let (Some(p), Some(from)) = (pointer, from) {
                let r = clip_rects[from].1;
                let ghost = Rect::from_min_size(Pos2::new(p.x - grab_dx, main_row.top() - 4.0), r.size());
                let fp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("reorder_ghost")));
                let fp = fp.with_clip_rect(content.expand(6.0));
                paint_video_clip(app, &fp, ghost, id, true, 0.9, &g);
                if let Some(ti) = target {
                    let x = boundary_x(&clip_rects, Some(id), ti);
                    painter.vline(x, main_row.y_range().expand(4.0), Stroke::new(2.5, ACCENT));
                }
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if !down {
                if let (Some(from), Some(mut to)) = (from, target) {
                    if to > from {
                        to -= 1;
                    }
                    if to != from {
                        let c = app.project.video.remove(from);
                        app.project.video.insert(to.min(app.project.video.len()), c);
                        app.invalidate_preview();
                    }
                }
                app.tl.drag = TlDrag::None;
            }
        }
    }

    // ------------------------------------------------- drop from the library
    let media_drag = egui::DragAndDrop::payload::<MediaDrag>(&ctx).map(|p| p.0);
    let audio_drag = egui::DragAndDrop::payload::<AudioDrag>(&ctx).map(|p| p.0);
    if let (Some(p), true) = (pointer, media_drag.is_some() || audio_drag.is_some()) {
        if body.contains(p) {
            let released = ctx.input(|i| i.pointer.any_released());
            if let Some(mid) = media_drag {
                let is_audio = app.project.media(mid).is_some_and(|m| m.kind == MediaKind::Audio);
                if is_audio {
                    painter.vline(p.x, audio_row.y_range(), Stroke::new(2.5, ACCENT_HI));
                } else {
                    let idx = insertion_index(&clip_rects, None, p.x);
                    let x = if clip_rects.is_empty() { content.left() + 2.0 } else { boundary_x(&clip_rects, None, idx) };
                    painter.vline(x, main_row.y_range().expand(4.0), Stroke::new(2.5, ACCENT_HI));
                    painter.rect_stroke(main_row, CornerRadius::same(6), Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Outside);
                }
                if released {
                    egui::DragAndDrop::clear_payload(&ctx);
                    if is_audio {
                        app.add_audio(mid, g.t(p.x).max(0.0));
                    } else {
                        app.add_to_main(mid, Some(insertion_index(&clip_rects, None, p.x)));
                    }
                }
            } else if let Some(mid) = audio_drag {
                painter.vline(p.x, audio_row.y_range(), Stroke::new(2.5, ACCENT_HI));
                if released {
                    egui::DragAndDrop::clear_payload(&ctx);
                    app.add_audio(mid, g.t(p.x).max(0.0));
                }
            }
        }
    }

    // ------------------------------------------------------------- overlays
    if let Some(s) = app.tl.snap_line {
        let x = g.x(s);
        painter.vline(x, Rect::from_min_max(ruler.left_bottom(), content.right_bottom()).y_range(), Stroke::new(1.0, GUIDE));
    }
    // Hover guide.
    if let Some(p) = pointer {
        if content.contains(p) && matches!(app.tl.drag, TlDrag::None) && !app.playing {
            painter.vline(p.x, Rect::from_min_max(ruler.left_top(), content.right_bottom()).y_range(), Stroke::new(1.0, Color32::from_white_alpha(28)));
        }
    }
    // Playhead.
    let px = g.x(app.playhead);
    if px >= content.left() - 1.0 && px <= content.right() + 1.0 {
        painter.vline(px, (ruler.top() + 4.0)..=content.bottom(), Stroke::new(1.6, PLAYHEAD));
        let head = Rect::from_center_size(Pos2::new(px, ruler.top() + 9.0), Vec2::new(12.0, 14.0));
        painter.rect_filled(head, CornerRadius::same(3), PLAYHEAD);
        let ph_resp = ui.interact(head.expand(4.0), ui.id().with("playhead"), Sense::drag());
        if ph_resp.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if ph_resp.drag_started() {
            app.tl.drag = TlDrag::Scrub;
        }
    }
}

fn edge_autoscroll(app: &mut KlipApp, g: &Geo, x: f32) {
    let edge = 30.0;
    if x > g.content.right() - edge {
        app.tl.scroll += 6.0 / g.pps;
    } else if x < g.content.left() + edge && app.tl.scroll > 0.0 {
        app.tl.scroll = (app.tl.scroll - 6.0 / g.pps).max(0.0);
    }
}

/// Index (in the full main-track list) before which a clip dropped at
/// screen `x` would be inserted. `skip` is the clip being dragged, if any.
fn insertion_index(rects: &[(Id, Rect)], skip: Option<Id>, x: f32) -> usize {
    let mut idx = 0;
    for (i, (id, r)) in rects.iter().enumerate() {
        if Some(*id) != skip && r.center().x < x {
            idx = i + 1;
        }
    }
    idx
}

fn boundary_x(rects: &[(Id, Rect)], _skip: Option<Id>, idx: usize) -> f32 {
    if idx == 0 {
        rects.first().map(|r| r.1.left()).unwrap_or(0.0)
    } else {
        rects.get(idx - 1).map(|r| r.1.right()).unwrap_or(0.0) + 1.0
    }
}

fn begin_drag(app: &KlipApp, kind: ClipKind, id: Id, mode: DragMode, t0: f64) -> TlDrag {
    let (start, src_in, len) = match kind {
        ClipKind::Video => app.project.video.iter().find(|c| c.id == id).map(|c| (0.0, c.src_in, c.src_out)).unwrap_or_default(),
        ClipKind::Text => app.project.texts.iter().find(|c| c.id == id).map(|c| (c.start, 0.0, c.dur)).unwrap_or_default(),
        ClipKind::Audio => app.project.audio.iter().find(|c| c.id == id).map(|c| (c.start, c.src_in, c.dur)).unwrap_or_default(),
    };
    TlDrag::Clip { kind, id, mode, t0, start, src_in, len }
}

/// Snap `t` to nearby edges. Returns the snapped time.
fn snap(app: &mut KlipApp, t: f64, skip: Id, g: &Geo) -> f64 {
    let thresh = 8.0 / g.pps;
    let mut pts = vec![0.0, app.playhead];
    let mut acc = 0.0;
    for c in &app.project.video {
        acc += c.dur();
        pts.push(acc);
    }
    for c in app.project.texts.iter().filter(|c| c.id != skip) {
        pts.extend([c.start, c.end()]);
    }
    for c in app.project.audio.iter().filter(|c| c.id != skip) {
        pts.extend([c.start, c.end()]);
    }
    let best = pts.into_iter().min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()));
    match best {
        Some(b) if (b - t).abs() < thresh => {
            app.tl.snap_line = Some(b);
            b
        }
        _ => t,
    }
}

fn apply_drag(app: &mut KlipApp, t: f64, g: &Geo) {
    let TlDrag::Clip { kind, id, mode, t0, start, src_in, len } = app.tl.drag else { return };
    let d = t - t0;
    let min = 1.0 / FPS * 3.0;
    match (kind, mode) {
        (ClipKind::Video, DragMode::TrimL) => {
            let (src_out, max_in) = (len, len - min);
            let speed = app.project.video.iter().find(|c| c.id == id).map_or(1.0, |c| c.speed());
            let mut v = (src_in + d * speed).clamp(0.0, max_in);
            // Keep the cut on a frame boundary.
            v = (v * FPS).round() / FPS;
            if let Some(c) = app.project.video.iter_mut().find(|c| c.id == id) {
                c.src_in = v.min(src_out - min);
            }
        }
        (ClipKind::Video, DragMode::TrimR) => {
            let i = app.project.video_index(id);
            let Some(i) = i else { return };
            let media = app.project.media(app.project.video[i].media).cloned();
            let max = media.map(|m| if m.kind == MediaKind::Image { 3600.0 } else { m.duration }).unwrap_or(len);
            // Snap the clip's right edge on the timeline.
            let start_t: f64 = app.project.video[..i].iter().map(|c| c.dur()).sum();
            let speed = app.project.video[i].speed();
            let end_t = snap(app, start_t + (len - src_in) / speed + d, id, g);
            let v = (src_in + (end_t - start_t) * speed).clamp(src_in + min, max);
            app.project.video[i].src_out = (v * FPS).round() / FPS;
        }
        (ClipKind::Video, DragMode::Move) => {}
        (ClipKind::Text, DragMode::Move) => {
            let mut s = (start + d).max(0.0);
            let s_snap = snap(app, s, id, g);
            if (s_snap - s).abs() > 1e-9 {
                s = s_snap;
            } else {
                let e = snap(app, s + len, id, g);
                s = (e - len).max(0.0);
            }
            if let Some(c) = app.project.texts.iter_mut().find(|c| c.id == id) {
                c.start = s;
            }
        }
        (ClipKind::Text, DragMode::TrimL) => {
            let end = start + len;
            let s = snap(app, (start + d).clamp(0.0, end - min), id, g).min(end - min);
            if let Some(c) = app.project.texts.iter_mut().find(|c| c.id == id) {
                c.start = s;
                c.dur = end - s;
            }
        }
        (ClipKind::Text, DragMode::TrimR) => {
            let e = snap(app, start + len + d, id, g);
            if let Some(c) = app.project.texts.iter_mut().find(|c| c.id == id) {
                c.dur = (e - start).max(min);
            }
        }
        (ClipKind::Audio, DragMode::Move) => {
            let mut s = (start + d).max(0.0);
            let s_snap = snap(app, s, id, g);
            if (s_snap - s).abs() > 1e-9 {
                s = s_snap;
            } else {
                let e = snap(app, s + len, id, g);
                s = (e - len).max(0.0);
            }
            if let Some(c) = app.project.audio.iter_mut().find(|c| c.id == id) {
                c.start = s;
            }
        }
        (ClipKind::Audio, DragMode::TrimL) => {
            let lo = (start - src_in).max(0.0); // can't reveal before source start
            let s = snap(app, (start + d).clamp(lo, start + len - min), id, g).clamp(lo, start + len - min);
            let delta = s - start;
            if let Some(c) = app.project.audio.iter_mut().find(|c| c.id == id) {
                c.start = s;
                c.src_in = src_in + delta;
                c.dur = len - delta;
            }
        }
        (ClipKind::Audio, DragMode::TrimR) => {
            let max = app
                .project
                .audio
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| app.project.media(c.media))
                .map(|m| m.duration - src_in)
                .unwrap_or(len);
            let e = snap(app, start + len + d, id, g);
            if let Some(c) = app.project.audio.iter_mut().find(|c| c.id == id) {
                c.dur = (e - start).clamp(min, max.max(min));
            }
        }
    }
}

fn selection_frame(p: &egui::Painter, r: Rect) {
    p.rect_stroke(r, CornerRadius::same(5), Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
    if r.width() > 24.0 {
        for x in [r.left() + 4.0, r.right() - 4.0] {
            let h = Rect::from_center_size(Pos2::new(x, r.center().y), Vec2::new(3.0, (r.height() * 0.45).min(18.0)));
            p.rect_filled(h, CornerRadius::same(2), Color32::WHITE);
        }
    }
}

fn paint_video_clip(app: &KlipApp, p: &egui::Painter, r: Rect, id: Id, sel: bool, alpha: f32, g: &Geo) {
    let Some(c) = app.project.video.iter().find(|c| c.id == id) else { return };
    let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
    p.rect_filled(r, CornerRadius::same(6), VIDEO_CLIP.gamma_multiply(alpha));
    let inner = r.shrink(1.0);
    let ip = p.with_clip_rect(inner.intersect(p.clip_rect()));
    if let Some(a) = app.assets.get(&c.media) {
        if let Some((_, first)) = a.strip.first() {
            let [tw, th] = first.size();
            let tile_w = (inner.height() * tw as f32 / th.max(1) as f32).max(8.0);
            let clip_vis = inner.intersect(p.clip_rect());
            let k0 = ((clip_vis.left() - inner.left()) / tile_w).floor().max(0.0) as i32;
            let mut k = k0;
            loop {
                let x = inner.left() + k as f32 * tile_w;
                if x > clip_vis.right() {
                    break;
                }
                let ts = c.src_at((x + tile_w / 2.0 - inner.left()) as f64 / g.pps);
                let tex = a
                    .strip
                    .iter()
                    .min_by(|(t1, _), (t2, _)| (t1 - ts).abs().total_cmp(&(t2 - ts).abs()))
                    .map(|(_, t)| t);
                if let Some(tex) = tex {
                    ip.image(tex.id(), Rect::from_min_size(Pos2::new(x, inner.top()), Vec2::new(tile_w, inner.height())), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
                }
                k += 1;
                if k - k0 > 400 {
                    break;
                }
            }
        }
    }
    // Label strip.
    let label_r = Rect::from_min_max(inner.min, Pos2::new(inner.right(), inner.top() + 18.0));
    ip.rect_filled(label_r, CornerRadius::ZERO, Color32::from_black_alpha((110.0 * alpha) as u8));
    let media = app.project.media(c.media);
    let name = media.map(|m| m.name.as_str()).unwrap_or("");
    let muted = media.is_some_and(|m| m.has_audio) && c.volume <= 0.001;
    let lx = inner.left().max(p.clip_rect().left()) + 6.0;
    ip.text(
        Pos2::new(lx, label_r.center().y),
        Align2::LEFT_CENTER,
        format!(
            "{}{}{}  {}",
            if muted { format!("{} ", ph::SPEAKER_SLASH) } else { String::new() },
            if (c.speed() - 1.0).abs() > 1e-3 { format!("{}× · ", fmt_speed(c.speed())) } else { String::new() },
            clip_len(c.dur()),
            elide(name, r.width() - 70.0, 6.2)
        ),
        FontId::proportional(10.5),
        Color32::WHITE.gamma_multiply(alpha),
    );
    round_mask(p, r, 6.0, ROW_BG);
    if sel {
        selection_frame(p, r);
    } else {
        p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, Color32::from_black_alpha(120)), StrokeKind::Inside);
    }
}

fn paint_ruler(p: &egui::Painter, g: &Geo, r: Rect) {
    let steps = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0];
    let major = steps.iter().copied().find(|s| s * g.pps >= 80.0).unwrap_or(7200.0);
    let minor = major / 5.0;
    let t0 = (g.scroll / minor).floor() * minor;
    let t1 = g.t(r.right());
    let mut t = t0;
    let mut n = (t0 / minor).round() as i64;
    while t <= t1 {
        let x = g.x(t);
        if x >= r.left() - 1.0 {
            let is_major = n % 5 == 0;
            let h = if is_major { 8.0 } else { 4.0 };
            p.vline(x, (r.bottom() - h)..=r.bottom(), Stroke::new(1.0, if is_major { TEXT_FAINT } else { BG4 }));
            if is_major {
                let label = if major < 1.0 {
                    format!("{t:.1}s")
                } else {
                    let s = t.round() as i64;
                    if s >= 3600 {
                        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
                    } else {
                        format!("{:02}:{:02}", s / 60, s % 60)
                    }
                };
                p.text(Pos2::new(x + 4.0, r.top() + 9.0), Align2::LEFT_CENTER, label, FontId::proportional(10.0), TEXT_DIM);
            }
        }
        n += 1;
        t = n as f64 * minor;
    }
}

fn toolbar_ui(app: &mut KlipApp, ui: &mut egui::Ui, r: Rect) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(Vec2::new(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = 2.0;
    let has_sel = app.selection != Selection::None;
    if icon_button(ui, ph::ARROW_COUNTER_CLOCKWISE, "Undo (Ctrl+Z)", app.can_undo()).clicked() {
        app.undo();
    }
    if icon_button(ui, ph::ARROW_CLOCKWISE, "Redo (Ctrl+Shift+Z)", app.can_redo()).clicked() {
        app.redo();
    }
    ui.add_space(6.0);
    ui.add(egui::Separator::default().vertical().spacing(6.0));
    ui.add_space(6.0);
    if icon_button(ui, ph::SCISSORS, "Split at playhead (S)", !app.project.video.is_empty()).clicked() {
        app.split();
    }
    if icon_button(ui, ph::COPY, "Duplicate (Ctrl+D)", has_sel).clicked() {
        app.duplicate_selection();
    }
    if icon_button(ui, ph::TRASH, "Delete (Del)", has_sel).clicked() {
        app.delete_selection();
    }
    ui.add_space(6.0);
    ui.add(egui::Separator::default().vertical().spacing(6.0));
    ui.add_space(6.0);
    if icon_button(ui, ph::TEXT_T, "Add text at playhead", true).clicked() {
        let mut t = crate::ui::library::text_presets()[3].1.clone();
        t.text = "Add text".into();
        app.add_text(t);
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(ui, ph::ARROWS_OUT_LINE_HORIZONTAL, "Fit timeline to window", true).clicked() {
            let total = app.project.duration().max(5.0);
            let w = (ui.ctx().content_rect().width() - HEADER_W - 80.0).max(200.0);
            app.tl.pps = (w / total as f32 * 0.95).clamp(app.tl.min_pps, MAX_PPS);
            app.tl.scroll = 0.0;
        }
        if icon_button(ui, ph::MAGNIFYING_GLASS_PLUS, "Zoom in (+)", true).clicked() {
            app.tl.pps = (app.tl.pps * 1.25).min(MAX_PPS);
        }
        let mut z = app.tl.pps.ln();
        if slider(ui, &mut z, app.tl.min_pps.ln()..=MAX_PPS.ln(), 110.0).changed() {
            app.tl.pps = z.exp();
        }
        if icon_button(ui, ph::MAGNIFYING_GLASS_MINUS, "Zoom out (-)", true).clicked() {
            app.tl.pps = (app.tl.pps / 1.25).max(app.tl.min_pps);
        }
    });
    ui.painter().hline(r.x_range().shrink(10.0), r.bottom(), Stroke::new(1.0, BORDER));
}

/// Short clip length for labels: "4.2s", or "35:32" / "1:02:10" when long.
fn clip_len(d: f64) -> String {
    if d < 60.0 {
        format!("{d:.1}s")
    } else {
        let s = d.round() as i64;
        if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
    }
}

/// 2, 1.5, 0.25
pub fn fmt_speed(s: f64) -> String {
    let t = format!("{s:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Shade the faded part of a clip and draw the volume ramp over it.
fn fade_curve(p: &egui::Painter, r: Rect, f: crate::model::AudioFades, g: &Geo) {
    let top = r.top() + 2.0;
    let bottom = r.bottom() - 2.0;
    let ramp = |x0: f32, x1: f32, rising: bool| {
        let (lo, hi) = if rising { (Pos2::new(x0, bottom), Pos2::new(x1, top)) } else { (Pos2::new(x0, top), Pos2::new(x1, bottom)) };
        // Dim the silent side of the ramp.
        let corner = if rising { Pos2::new(x0, top) } else { Pos2::new(x1, top) };
        p.add(egui::Shape::convex_polygon(vec![lo, corner, hi], Color32::from_black_alpha(120), Stroke::NONE));
        p.line_segment([lo, hi], Stroke::new(1.5, Color32::WHITE.gamma_multiply(0.9)));
        // Knob where the fade meets full volume.
        p.circle_filled(if rising { hi } else { lo }, 3.5, Color32::WHITE);
    };
    if f.fade_in > 0.0 {
        ramp(r.left(), (r.left() + f.fade_in * g.pps as f32).min(r.right()), true);
    }
    if f.fade_out > 0.0 {
        ramp((r.right() - f.fade_out * g.pps as f32).max(r.left()), r.right(), false);
    }
}
