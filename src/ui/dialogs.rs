//! Home screen dialogs: new project, rename, delete forever.

use crate::app::{KlipApp, NewProject};
use crate::model::Aspect;
use crate::theme::*;
use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;

/// A centred modal card. Returns true when dismissed (Esc / backdrop).
fn modal(ctx: &egui::Context, id: &str, icon: &str, title: &str, width: f32, body: impl FnOnce(&mut egui::Ui)) -> bool {
    egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .stroke(Stroke::new(1.0, BORDER_HI))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(Margin::same(24)),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(R_MD), ACCENT.gamma_multiply(0.16));
                ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), ACCENT);
                ui.add_space(4.0);
                ui.label(RichText::new(title).size(16.0).family(semibold_family()).color(TEXT));
            });
            ui.add_space(16.0);
            body(ui);
        })
        .should_close()
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(TEXT_DIM));
    ui.add_space(4.0);
}

pub fn dialogs(app: &mut KlipApp, ctx: &egui::Context) {
    new_project(app, ctx);
    rename(app, ctx);
    confirm_delete(app, ctx);
}

fn new_project(app: &mut KlipApp, ctx: &egui::Context) {
    let Some(np) = &mut app.home.new_project else { return };
    let mut act = 0;
    let title = if np.media.is_empty() { "New project".to_string() } else { format!("New project with {} file(s)", np.media.len()) };
    const W: f32 = 480.0;
    let esc = modal(ctx, "new_project", ph::FILM_SLATE, &title, W, |ui| {
        field_label(ui, "Name");
        let r = text_input(ui, &mut np.name, W);
        if !r.has_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            act = 1;
        }
        ui.add_space(16.0);
        field_label(ui, "Canvas");
        ratio_cards(ui, &mut np.aspect, W);
        ui.add_space(22.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, "Create").clicked() {
                act = 1;
            }
            if secondary_button(ui, "Cancel").clicked() {
                act = 2;
            }
        });
    });
    if act == 1 {
        let NewProject { name, aspect, media } = app.home.new_project.take().unwrap();
        app.create_project(&name, aspect, media);
    } else if act == 2 || esc {
        app.home.new_project = None;
    }
}

/// Four cards, each with a little frame of the canvas shape.
fn ratio_cards(ui: &mut egui::Ui, aspect: &mut Aspect, width: f32) {
    let gap = 10.0;
    let w = (width - gap * 3.0) / 4.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for a in Aspect::ALL {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 104.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            let sel = *aspect == a;
            let h = anim(ui, ("ratio_h", a.label()), resp.hovered());
            let s = anim(ui, ("ratio_s", a.label()), sel);
            let p = ui.painter();
            p.rect_filled(rect, CornerRadius::same(R_MD), mix(BG2, BG3, h * 0.6));
            p.rect_stroke(rect, CornerRadius::same(R_MD), Stroke::new(1.0 + s * 0.5, mix(BORDER, ACCENT, s)), StrokeKind::Inside);
            // Canvas shape, fitted in a 44 px box.
            let ar = crate::app::aspect_of(a);
            let box_ = 40.0;
            let size = if ar < 1.0 { Vec2::new(box_ * ar, box_) } else { Vec2::new(box_, box_ / ar) };
            let shape = Rect::from_center_size(rect.center_top() + Vec2::new(0.0, 38.0), size);
            p.rect_stroke(shape, CornerRadius::same(4), Stroke::new(1.5, mix(TEXT_FAINT, ACCENT, s)), StrokeKind::Inside);
            p.text(rect.center_bottom() - Vec2::new(0.0, 32.0), Align2::CENTER_CENTER, a.label(), FontId::new(13.0, medium_family()), mix(TEXT_DIM, TEXT, s));
            p.text(rect.center_bottom() - Vec2::new(0.0, 15.0), Align2::CENTER_CENTER, ratio_use(a), FontId::proportional(11.0), TEXT_FAINT);
            if resp.clicked() {
                *aspect = a;
            }
        }
    });
}

fn ratio_use(a: Aspect) -> &'static str {
    match a {
        Aspect::Reel => "Reels, TikTok",
        Aspect::Portrait => "Feed post",
        Aspect::Square => "Square",
        Aspect::Landscape => "YouTube",
    }
}

fn rename(app: &mut KlipApp, ctx: &egui::Context) {
    let Some((id, name)) = &mut app.home.rename else { return };
    let mut act = 0;
    let esc = modal(ctx, "rename", ph::PENCIL_SIMPLE, "Rename project", 380.0, |ui| {
        let r = text_input(ui, name, 380.0);
        if !r.has_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            act = 1;
        }
        ui.add_space(20.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, "Rename").clicked() {
                act = 1;
            }
            if secondary_button(ui, "Cancel").clicked() {
                act = 2;
            }
        });
    });
    if act == 1 {
        let (id, name) = (id.clone(), name.clone());
        match app.store.rename(&id, &name) {
            Ok(_) => {
                app.home.rename = None;
                app.refresh_home();
            }
            Err(e) => app.toast(crate::app::ToastKind::Error, e),
        }
    } else if act == 2 || esc {
        app.home.rename = None;
    }
}

fn confirm_delete(app: &mut KlipApp, ctx: &egui::Context) {
    let Some(m) = &app.home.confirm_delete else { return };
    let body = format!("“{}” will be removed permanently. This can't be undone.", m.name);
    let mut act = 0;
    let esc = modal(ctx, "delete_forever", ph::TRASH, "Delete forever?", 380.0, |ui| {
        ui.label(RichText::new(body).size(13.5).color(TEXT_DIM));
        ui.add_space(20.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if rich_button(ui, "Delete forever", Kind::Danger, 0.0).clicked() {
                act = 1;
            }
            if secondary_button(ui, "Cancel").clicked() {
                act = 2;
            }
        });
    });
    if act == 1 {
        let id = app.home.confirm_delete.take().unwrap().id;
        if let Err(e) = app.store.delete_forever(&id) {
            app.toast(crate::app::ToastKind::Error, format!("Could not delete: {e}"));
        }
        app.refresh_home();
    } else if act == 2 || esc {
        app.home.confirm_delete = None;
    }
}
