//! Small confirmation dialogs: unsaved changes on close, session restore.

use crate::app::KlipApp;
use crate::theme::*;
use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke, Vec2};
use egui_phosphor::regular as ph;

fn dialog(ctx: &egui::Context, id: &str, icon: &str, title: &str, body: &str, buttons: impl FnOnce(&mut egui::Ui)) -> bool {
    let r = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .stroke(Stroke::new(1.0, BORDER_HI))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(Margin::same(24)),
        )
        .show(ctx, |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(R_MD), ACCENT.gamma_multiply(0.16));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(18.0), ACCENT);
                ui.add_space(4.0);
                ui.label(RichText::new(title).size(16.0).family(semibold_family()).color(TEXT));
            });
            ui.add_space(10.0);
            ui.label(RichText::new(body).size(13.5).color(TEXT_DIM));
            ui.add_space(18.0);
            ui.with_layout(Layout::right_to_left(Align::Center), buttons);
        });
    r.should_close()
}

pub fn dialogs(app: &mut KlipApp, ctx: &egui::Context) {
    if app.close_prompt {
        let body = format!("“{}” has changes that haven't been saved. Save them before closing?", app.project.name);
        let mut act = 0;
        let esc = dialog(ctx, "close_prompt", ph::FLOPPY_DISK_BACK, "Save changes?", &body, |ui| {
            if primary_button(ui, "Save").clicked() {
                act = 1;
            }
            if secondary_button(ui, "Cancel").clicked() {
                act = 2;
            }
            ui.add_space(8.0);
            if secondary_button(ui, "Don't save").clicked() {
                act = 3;
            }
        });
        match act {
            1 => app.close_save(),
            3 => app.close_discard(),
            2 => app.close_prompt = false,
            _ if esc => app.close_prompt = false,
            _ => {}
        }
        return;
    }

    if let Some((p, _)) = &app.restore_offer {
        let body = format!(
            "Klip closed before “{}” was saved. Would you like to pick up where you left off?",
            p.name
        );
        let mut act = 0;
        dialog(ctx, "restore", ph::CLOCK_COUNTER_CLOCKWISE, "Restore last session?", &body, |ui| {
            if primary_button(ui, "Restore").clicked() {
                act = 1;
            }
            if secondary_button(ui, "Discard").clicked() {
                act = 2;
            }
        });
        match act {
            1 => app.restore_session(),
            2 => app.discard_session(),
            _ => {}
        }
    }
}
