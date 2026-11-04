//! Window layout:
//!
//!   ┌──────────────── top bar ────────────────┐
//!   │ library │      preview       │ inspector │
//!   ├─────────────── timeline ────────────────┤

pub mod export_dialog;
mod inspector;
mod library;
mod preview;
mod timeline;

use crate::app::{KlipApp, ToastKind};
use crate::model::Aspect;
use crate::theme::*;
use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Vec2};
use egui_phosphor::regular as ph;

pub fn draw(app: &mut KlipApp, ui: &mut egui::Ui) {
    let gap = 6;
    if app.fullscreen {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(Color32::BLACK))
            .show(ui, |ui| preview::preview(app, ui));
        toasts(app, ui.ctx());
        return;
    }
    let screen_h = ui.ctx().content_rect().height();
    egui::Panel::top("topbar")
        .exact_size(58.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(BG0).inner_margin(Margin::symmetric(14, 0)))
        .show(ui, |ui| top_bar(app, ui));

    egui::Panel::bottom("timeline")
        .resizable(true)
        .default_size((screen_h * 0.34).clamp(232.0, 320.0))
        .size_range(200.0..=(screen_h * 0.6).max(240.0))
        .show_separator_line(false)
        .frame(Frame::new().fill(BG0).inner_margin(Margin { left: gap, right: gap, top: 0, bottom: gap }))
        .show(ui, |ui| card(ui, Margin::same(0), |ui| timeline::timeline(app, ui)));

    egui::Panel::left("library")
        .resizable(true)
        .default_size(330.0)
        .size_range(280.0..=480.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(BG0).inner_margin(Margin { left: gap, right: 0, top: 0, bottom: gap }))
        .show(ui, |ui| card(ui, Margin::same(0), |ui| library::library(app, ui)));

    egui::Panel::right("inspector")
        .resizable(true)
        .default_size(310.0)
        .size_range(270.0..=440.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(BG0).inner_margin(Margin { left: 0, right: gap, top: 0, bottom: gap }))
        .show(ui, |ui| card(ui, Margin::same(0), |ui| inspector::inspector(app, ui)));

    egui::CentralPanel::default()
        .frame(Frame::new().fill(BG0).inner_margin(Margin { left: gap, right: gap, top: 0, bottom: gap }))
        .show(ui, |ui| card(ui, Margin::same(0), |ui| preview::preview(app, ui)));

    export_dialog::export_dialog(app, ui.ctx());
    os_drop_overlay(ui.ctx());
    toasts(app, ui.ctx());
}

/// Rounded surface that fills the space it's given.
pub fn card<R>(ui: &mut egui::Ui, margin: Margin, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(BG1)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            add(ui)
        })
        .inner
}

fn top_bar(app: &mut KlipApp, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), egui::Sense::hover());
        paint_logo(ui.painter(), r);
        ui.add_space(2.0);
        ui.label(RichText::new("klip").size(20.0).family(heading_family()).color(TEXT));
        ui.add_space(14.0);

        ui.add(egui::Separator::default().vertical().spacing(4.0));
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        // Hug the text so the pencil sits right next to the name.
        let text_w = ui
            .painter()
            .layout_no_wrap(app.project.name.clone(), egui::FontId::proportional(14.0), TEXT)
            .size()
            .x;
        let name = inline_input(ui, &mut app.project.name, (text_w + 50.0).clamp(110.0, 340.0));
        let focused = name.has_focus();
        let hovered = name.hovered();
        name.on_hover_text("Click to rename project");
        let t = anim(ui, "rename_icon", hovered || focused);
        ui.label(RichText::new(ph::PENCIL_SIMPLE).size(14.0).color(mix(TEXT_FAINT, ACCENT, t)));
        if app.is_dirty() {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 3.5, TEXT_FAINT);
            resp.on_hover_text("Unsaved changes");
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if primary_button(ui, format!("{}  Export", ph::UPLOAD_SIMPLE)).on_hover_text("Ctrl+E").clicked() {
                app.export.open = true;
            }
            if secondary_button(ui, format!("{}  Save", ph::FLOPPY_DISK_BACK)).on_hover_text("Save project (Ctrl+S)").clicked() {
                app.save();
            }
            if secondary_button(ui, format!("{}  Open", ph::FOLDER_SIMPLE)).on_hover_text("Open project (Ctrl+O)").clicked() {
                app.dialog_open();
            }
            ui.add_space(8.0);
            ui.add(egui::Separator::default().vertical().spacing(4.0));
            ui.add_space(4.0);
            // Canvas ratio switch.
            let opts: Vec<(Aspect, &str)> = Aspect::ALL.iter().map(|a| (*a, a.label())).collect();
            segmented(ui, &mut app.project.aspect, &opts);
            ui.add_space(2.0);
            ui.label(RichText::new(ph::FRAME_CORNERS).size(17.0).color(TEXT_DIM)).on_hover_text("Canvas size");
        });
    });
}

fn os_drop_overlay(ctx: &egui::Context) {
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if !hovering {
        return;
    }
    let screen = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_overlay")));
    p.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(170));
    let r = screen.shrink(40.0);
    p.rect_stroke(r, CornerRadius::same(14), Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
    p.text(r.center() - Vec2::new(0.0, 18.0), egui::Align2::CENTER_CENTER, ph::DOWNLOAD_SIMPLE, egui::FontId::proportional(44.0), ACCENT_HI);
    p.text(
        r.center() + Vec2::new(0.0, 30.0),
        egui::Align2::CENTER_CENTER,
        "Drop to import",
        egui::FontId::new(22.0, heading_family()),
        TEXT,
    );
}

fn toasts(app: &mut KlipApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    app.toasts.retain(|t| t.until > now);
    if app.toasts.is_empty() && app.importing == 0 {
        return;
    }
    let mut items: Vec<(String, ToastKind, f32)> = app
        .toasts
        .iter()
        .map(|t| (t.msg.clone(), t.kind, ((t.until - now) as f32 / 0.3).clamp(0.0, 1.0)))
        .collect();
    if app.importing > 0 {
        items.push((format!("Importing {} file(s)…", app.importing), ToastKind::Info, 1.0));
    }
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 64.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            for (msg, kind, alpha) in items {
                let (icon, col) = match kind {
                    ToastKind::Info => (ph::INFO, ACCENT_HI),
                    ToastKind::Success => (ph::CHECK_CIRCLE, SUCCESS),
                    ToastKind::Error => (ph::WARNING_CIRCLE, DANGER),
                };
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(BG3)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(R_MD))
                    .inner_margin(Margin::symmetric(16, 10))
                    .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(120) })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon).color(col).size(16.0));
                            ui.label(RichText::new(msg).color(TEXT));
                        });
                    });
                ui.add_space(6.0);
            }
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}
