//! Home screen: the grid of projects, shown when Klip starts.

use crate::app::{HomeSort, KlipApp, NewProject, ToastKind};
use crate::model::Aspect;
use crate::store::{self, Meta};
use crate::theme::*;
use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ph;

const CARD_W: f32 = 212.0;
const GAP: f32 = 20.0;
const UNDO_SECS: f64 = 6.0;

pub fn home(app: &mut KlipApp, ui: &mut egui::Ui) {
    egui::Panel::top("home_bar")
        .exact_size(58.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(BG0).inner_margin(Margin::symmetric(14, 0)))
        .show(ui, |ui| top_bar(app, ui));

    egui::CentralPanel::default()
        .frame(Frame::new().fill(BG0).inner_margin(Margin { left: 6, right: 6, top: 0, bottom: 6 }))
        .show(ui, |ui| {
            super::card(ui, Margin::same(0), |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    Frame::new().inner_margin(Margin { left: 36, right: 36, top: 30, bottom: 36 }).show(ui, |ui| {
                        if app.home.show_trash {
                            trash(app, ui);
                        } else {
                            projects(app, ui);
                        }
                    });
                });
            });
        });

    undo_bar(app, ui.ctx());
}

fn top_bar(app: &mut KlipApp, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::hover());
        paint_logo(ui.painter(), r);
        ui.add_space(2.0);
        ui.label(RichText::new("klip").size(20.0).family(heading_family()).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if primary_button(ui, format!("{}  New project", ph::PLUS)).on_hover_text("Ctrl+N").clicked() {
                open_new(app, vec![]);
            }
            if secondary_button(ui, format!("{}  Import project", ph::DOWNLOAD_SIMPLE))
                .on_hover_text("Add a .klip file (Ctrl+O)")
                .clicked()
            {
                app.dialog_import_project();
            }
        });
    });
}

fn open_new(app: &mut KlipApp, media: Vec<std::path::PathBuf>) {
    app.home.new_project = Some(NewProject { name: app.store.unique_name("Untitled reel"), aspect: Aspect::Reel, media });
}

// ------------------------------------------------------------------ projects

fn projects(app: &mut KlipApp, ui: &mut egui::Ui) {
    let total = app.home.projects.len();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Projects").size(24.0).family(heading_family()).color(TEXT));
        if total > 0 {
            ui.add_space(4.0);
            ui.label(RichText::new(total.to_string()).size(15.0).color(TEXT_FAINT));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let n = app.home.trash.len();
            let tip = if n == 0 { "Trash is empty".to_string() } else { format!("Trash ({n})") };
            if icon_button(ui, ph::TRASH, &tip, true).clicked() {
                app.home.show_trash = true;
                app.home.menu = None;
            }
            if total > 1 {
                segmented(ui, &mut app.home.sort, &[(HomeSort::Recent, "Recent"), (HomeSort::Name, "Name")]);
                search(ui, &mut app.home.search);
            }
        });
    });
    ui.add_space(22.0);

    if total == 0 {
        empty_state(app, ui);
        return;
    }

    let q = app.home.search.trim().to_lowercase();
    let mut list: Vec<Meta> = app.home.projects.iter().filter(|m| q.is_empty() || m.name.to_lowercase().contains(&q)).cloned().collect();
    if app.home.sort == HomeSort::Name {
        list.sort_by_key(|m| m.name.to_lowercase());
    }
    if list.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ph::MAGNIFYING_GLASS).size(28.0).color(TEXT_FAINT));
            ui.add_space(6.0);
            ui.label(RichText::new(format!("No projects match “{}”", app.home.search.trim())).color(TEXT_DIM));
        });
        return;
    }

    let mut anchor = None;
    grid(ui, list.len() + 1, |ui, i, size| {
        if i == 0 {
            new_tile(app, ui, size);
        } else if let Some(r) = project_card(app, ui, &list[i - 1], size, false) {
            anchor = Some(r);
        }
    });
    menu(app, ui.ctx(), anchor, false);
}

fn trash(app: &mut KlipApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if icon_button(ui, ph::ARROW_LEFT, "Back to projects", true).clicked() {
            app.home.show_trash = false;
            app.home.menu = None;
        }
        ui.add_space(2.0);
        ui.label(RichText::new("Trash").size(24.0).family(heading_family()).color(TEXT));
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Projects here are deleted for good after 30 days.").color(TEXT_FAINT));
    ui.add_space(22.0);
    let list = app.home.trash.clone();
    if list.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ph::TRASH).size(30.0).color(TEXT_FAINT));
            ui.add_space(6.0);
            ui.label(RichText::new("Trash is empty").color(TEXT_DIM));
        });
        return;
    }
    let mut anchor = None;
    grid(ui, list.len(), |ui, i, size| {
        if let Some(r) = project_card(app, ui, &list[i], size, true) {
            anchor = Some(r);
        }
    });
    menu(app, ui.ctx(), anchor, true);
}

/// Lay `n` equal cells out in rows that fill the width.
fn grid(ui: &mut egui::Ui, n: usize, mut cell: impl FnMut(&mut egui::Ui, usize, Vec2)) {
    let avail = ui.available_width();
    let cols = (((avail + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor().min(CARD_W * 1.25);
    let size = Vec2::new(w, w * 1.2 + 52.0);
    for row in 0..n.div_ceil(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for i in row * cols..((row + 1) * cols).min(n) {
                cell(ui, i, size);
            }
        });
        ui.add_space(GAP);
    }
}

fn search(ui: &mut egui::Ui, text: &mut String) {
    let r = text_input(ui, text, 240.0);
    if text.is_empty() && !r.has_focus() {
        let p = ui.painter();
        let y = r.rect.center().y;
        p.text(Pos2::new(r.rect.left() + 13.0, y), Align2::LEFT_CENTER, ph::MAGNIFYING_GLASS, FontId::proportional(14.0), TEXT_FAINT);
        p.text(Pos2::new(r.rect.left() + 34.0, y), Align2::LEFT_CENTER, "Search projects", FontId::proportional(13.5), TEXT_FAINT);
    }
}

fn empty_state(app: &mut KlipApp, ui: &mut egui::Ui) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(16), ACCENT.gamma_multiply(0.14));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::FILM_SLATE, FontId::proportional(30.0), ACCENT);
        ui.add_space(16.0);
        ui.label(RichText::new("No projects yet").size(18.0).family(semibold_family()).color(TEXT));
        ui.add_space(4.0);
        ui.label(RichText::new("Create a project to start editing, or drop videos anywhere here.").color(TEXT_DIM));
        ui.add_space(18.0);
        if primary_button(ui, format!("{}  New project", ph::PLUS)).clicked() {
            open_new(app, vec![]);
        }
    });
}

/// The first cell of the grid: a dashed "New project" tile.
fn new_tile(app: &mut KlipApp, ui: &mut egui::Ui, size: Vec2) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let h = anim(ui, "new_tile", resp.hovered());
    let thumb = Rect::from_min_size(rect.min, Vec2::new(size.x, size.x * 1.2));
    let p = ui.painter();
    p.rect_filled(thumb, CornerRadius::same(12), mix(BG1, BG2, h));
    dashed_rrect(p, thumb.shrink(0.5), 12.0, Stroke::new(1.2, mix(BORDER_HI, ACCENT, h)));
    let c = thumb.center() - Vec2::new(0.0, 14.0);
    p.circle_filled(c, 24.0, mix(BG3, ACCENT, h));
    p.text(c, Align2::CENTER_CENTER, ph::PLUS, FontId::new(20.0, bold_icons()), mix(TEXT, ON_ACCENT, h));
    p.text(c + Vec2::new(0.0, 44.0), Align2::CENTER_CENTER, "New project", FontId::new(14.0, medium_family()), TEXT);
    if resp.clicked() {
        open_new(app, vec![]);
    }
}

/// One project card. Returns the "⋯" button rect when its menu is open.
fn project_card(app: &mut KlipApp, ui: &mut egui::Ui, m: &Meta, size: Vec2, in_trash: bool) -> Option<Rect> {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    app.load_cover(m);
    let key = m.id.as_str();
    let menu_open = app.home.menu.as_deref() == Some(key);
    let hov_b = resp.hovered() || menu_open;
    let h = anim(ui, ("card_h", key), hov_b);
    let thumb = Rect::from_min_size(rect.min, Vec2::new(size.x, size.x * 1.2));
    let p = ui.painter();
    p.rect_filled(thumb, CornerRadius::same(12), BG2);

    // Cover, fitted inside the tile in its own ratio.
    let ar = crate::app::aspect_of(m.aspect);
    let inner = thumb.shrink(10.0);
    let fit = if ar < inner.aspect_ratio() { Vec2::new(inner.height() * ar, inner.height()) } else { Vec2::new(inner.width(), inner.width() / ar) };
    let cover = Rect::from_center_size(inner.center(), fit);
    let dim = if in_trash { 0.55 } else { 1.0 };
    match app.home.thumbs.get(key) {
        Some((_, tex)) => {
            ui.painter().add(
                egui::epaint::RectShape::filled(cover, CornerRadius::same(6), Color32::WHITE.gamma_multiply(dim))
                    .with_texture(tex.id(), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
            );
        }
        None => {
            p.rect_filled(cover, CornerRadius::same(6), BG3);
            p.text(cover.center(), Align2::CENTER_CENTER, ph::FILM_STRIP, FontId::proportional(24.0), TEXT_FAINT);
        }
    }
    let p = ui.painter();
    // Ratio and length badges.
    let badge = |text: String, right: bool| {
        let g = p.layout_no_wrap(text, FontId::new(11.0, medium_family()), Color32::WHITE);
        let at = if right {
            cover.right_bottom() - Vec2::new(8.0 + g.size().x + 10.0, 8.0 + g.size().y + 4.0)
        } else {
            cover.left_bottom() + Vec2::new(8.0, -(8.0 + g.size().y + 4.0))
        };
        let r = Rect::from_min_size(at, g.size() + Vec2::new(10.0, 4.0));
        p.rect_filled(r, CornerRadius::same(5), Color32::from_black_alpha(165));
        p.galley(r.min + Vec2::new(5.0, 2.0), g, Color32::WHITE);
    };
    badge(m.aspect.label().to_string(), false);
    if m.duration > 0.0 {
        badge(length(m.duration), true);
    }
    let ring = mix(Color32::TRANSPARENT, BORDER_HI, h);
    let ring = if menu_open { ACCENT.gamma_multiply(0.8) } else { ring };
    if ring.a() > 0 {
        p.rect_stroke(thumb, CornerRadius::same(12), Stroke::new(1.0, ring), StrokeKind::Inside);
    }

    // Name and details.
    let name_y = thumb.bottom() + 20.0;
    p.text(
        Pos2::new(rect.left() + 2.0, name_y),
        Align2::LEFT_CENTER,
        super::library::elide(&m.name, size.x - 4.0, 7.6),
        FontId::new(14.0, medium_family()),
        TEXT,
    );
    let sub = match (in_trash, m.deleted) {
        (true, Some(d)) => {
            let left = store::TRASH_TTL.saturating_sub(store::now().saturating_sub(d)).div_ceil(86_400);
            format!("Deleted {} · {} left", ago(d), plural(left.max(1), "day"))
        }
        _ => format!("Edited {}", ago(m.edited)),
    };
    p.text(Pos2::new(rect.left() + 2.0, name_y + 20.0), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), TEXT_FAINT);

    // "⋯" menu button, shown on hover.
    let dots_r = Rect::from_center_size(thumb.right_top() + Vec2::new(-22.0, 22.0), Vec2::splat(30.0));
    let dots = ui.interact(dots_r, ui.id().with(("dots", key)), Sense::click());
    let show = anim(ui, ("dots_v", key), hov_b || dots.hovered());
    if show > 0.01 {
        let dh = anim(ui, ("dots_h", key), dots.hovered() || menu_open);
        let p = ui.painter();
        p.circle_filled(dots_r.center(), 15.0, mix(Color32::from_black_alpha(150), Color32::from_black_alpha(215), dh).gamma_multiply(show));
        p.text(dots_r.center(), Align2::CENTER_CENTER, ph::DOTS_THREE, FontId::new(18.0, bold_icons()), Color32::WHITE.gamma_multiply(show));
    }
    let dots = dots.on_hover_cursor(egui::CursorIcon::PointingHand);
    if dots.clicked() {
        app.home.menu = if menu_open { None } else { Some(m.id.clone()) };
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("home_menu_skip"), true));
    } else if resp.clicked() && !in_trash {
        app.open_project(&m.id);
    } else if resp.secondary_clicked() {
        app.home.menu = Some(m.id.clone());
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("home_menu_skip"), true));
    }
    if resp.hovered() && !dots.hovered() && !in_trash {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (app.home.menu.as_deref() == Some(key)).then_some(dots_r)
}

// ---------------------------------------------------------------------- menu

fn menu(app: &mut KlipApp, ctx: &egui::Context, anchor: Option<Rect>, in_trash: bool) {
    let Some(id) = app.home.menu.clone() else { return };
    let Some(anchor) = anchor else {
        // Card scrolled away or filtered out.
        app.home.menu = None;
        return;
    };
    let Some(meta) = app.home.projects.iter().chain(&app.home.trash).find(|m| m.id == id).cloned() else {
        app.home.menu = None;
        return;
    };
    let mut act = "";
    let area = egui::Area::new(egui::Id::new("home_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom() + Vec2::new(-170.0, 6.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(BG2)
                .stroke(Stroke::new(1.0, BORDER_HI))
                .corner_radius(CornerRadius::same(R_MD))
                .inner_margin(Margin::same(5))
                .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
                .show(ui, |ui| {
                    ui.set_width(196.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let items: &[(&str, &str, &str)] = if in_trash {
                        &[("restore", ph::ARROW_COUNTER_CLOCKWISE, "Restore"), ("-", "", ""), ("forever", ph::TRASH, "Delete forever")]
                    } else {
                        &[
                            ("open", ph::FOLDER_OPEN, "Open"),
                            ("rename", ph::PENCIL_SIMPLE, "Rename"),
                            ("dup", ph::COPY, "Duplicate"),
                            ("show", ph::FOLDER_SIMPLE, "Show in folder"),
                            ("-", "", ""),
                            ("delete", ph::TRASH, "Move to trash"),
                        ]
                    };
                    for (key, icon, label) in items {
                        if *key == "-" {
                            ui.add_space(3.0);
                            let y = ui.cursor().top();
                            ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, BORDER));
                            ui.add_space(4.0);
                            continue;
                        }
                        let danger = matches!(*key, "delete" | "forever");
                        if menu_item(ui, icon, label, danger) {
                            act = key;
                        }
                    }
                });
        });

    let skip = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("home_menu_skip")).unwrap_or(false));
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if (outside && !skip) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.home.menu = None;
    }
    if act.is_empty() {
        return;
    }
    app.home.menu = None;
    let s = &app.store;
    match act {
        "open" => app.open_project(&id),
        "rename" => app.home.rename = Some((id, meta.name.clone())),
        "dup" => match s.duplicate(&id) {
            Ok(_) => app.refresh_home(),
            Err(e) => app.toast(ToastKind::Error, e),
        },
        "show" => {
            let _ = std::process::Command::new("xdg-open").arg(s.dir(&id)).spawn();
        }
        "delete" => match s.delete(&id) {
            Ok(()) => {
                app.home.undo_delete = Some((meta, ctx.input(|i| i.time) + UNDO_SECS));
                app.refresh_home();
            }
            Err(e) => app.toast(ToastKind::Error, format!("Could not delete: {e}")),
        },
        "restore" => match s.restore(&id) {
            Ok(m) => {
                app.refresh_home();
                app.toast(ToastKind::Success, format!("“{}” restored", m.name));
            }
            Err(e) => app.toast(ToastKind::Error, format!("Could not restore: {e}")),
        },
        "forever" => app.home.confirm_delete = Some(meta),
        _ => {}
    }
}

fn menu_item(ui: &mut egui::Ui, icon: &str, label: &str, danger: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let h = anim(ui, ("menu_item", label), resp.hovered());
    let p = ui.painter();
    if h > 0.01 {
        p.rect_filled(rect, CornerRadius::same(R_SM - 2), BG4.gamma_multiply(h));
    }
    let col = if danger { DANGER } else { mix(TEXT_DIM, TEXT, h) };
    p.text(rect.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(16.0), if danger { DANGER } else { TEXT_DIM });
    p.text(rect.left_center() + Vec2::new(38.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), if danger { col } else { mix(TEXT, Color32::WHITE, h) });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// "Moved to trash · Undo" bar at the bottom after a delete.
fn undo_bar(app: &mut KlipApp, ctx: &egui::Context) {
    let Some((meta, until)) = &app.home.undo_delete else { return };
    let now = ctx.input(|i| i.time);
    if now > *until {
        app.home.undo_delete = None;
        return;
    }
    let alpha = ((*until - now) as f32 / 0.3).clamp(0.0, 1.0);
    let msg = format!("“{}” moved to trash", super::library::elide(&meta.name, 260.0, 7.0));
    let mut undo = false;
    egui::Area::new(egui::Id::new("undo_delete"))
        .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -28.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_opacity(alpha);
            Frame::new()
                .fill(BG3)
                .stroke(Stroke::new(1.0, BORDER_HI))
                .corner_radius(CornerRadius::same(R_MD))
                .inner_margin(Margin { left: 16, right: 6, top: 6, bottom: 6 })
                .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(120) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ph::TRASH).size(15.0).color(TEXT_DIM));
                        ui.label(RichText::new(msg).color(TEXT));
                        ui.add_space(10.0);
                        undo = link_button(ui, "Undo").clicked();
                    });
                });
        });
    if undo {
        let id = meta.id.clone();
        app.home.undo_delete = None;
        if app.store.restore(&id).is_ok() {
            app.refresh_home();
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}

// -------------------------------------------------------------------- dates

/// 0:42, 12:05, 1:04:39
fn length(secs: f64) -> String {
    let s = secs.round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") }
}

/// "just now", "5 min ago", "yesterday", "12 Oct"…
pub fn ago(t: u64) -> String {
    let d = store::now().saturating_sub(t);
    match d {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", d / 60),
        3600..86_400 => format!("{} ago", plural(d / 3600, "hour")),
        86_400..172_800 => "yesterday".into(),
        172_800..604_800 => format!("{} days ago", d / 86_400),
        _ => {
            let (y, m, day) = civil(t / 86_400);
            let (ny, _, _) = civil(store::now() / 86_400);
            let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m as usize - 1];
            if y == ny { format!("{day} {mon}") } else { format!("{day} {mon} {y}") }
        }
    }
}

/// Days since 1970-01-01 → (year, month, day). (Howard Hinnant's algorithm.)
fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_dates() {
        assert_eq!(super::civil(0), (1970, 1, 1));
        assert_eq!(super::civil(20_767), (2026, 11, 10));
        assert_eq!(super::civil(11_016), (2000, 2, 29));
    }
}
