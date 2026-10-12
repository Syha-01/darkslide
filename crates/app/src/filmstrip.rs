//! Filmstrip: thumbnails, ratings, labels, selection, filtering.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use egui_phosphor::regular as ic;

use crate::app::App;
use crate::doc::ColorLabel;

pub fn filmstrip(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} images", app.docs.len())).small().weak());
        ui.separator();
        ui.label(RichText::new("Filter:").small());
        egui::ComboBox::from_id_salt("frating")
            .width(60.0)
            .selected_text(if app.filter_rating == 0 { "All".into() } else { format!("≥ {}★", app.filter_rating) })
            .show_ui(ui, |ui| {
                for r in 0..=5 {
                    ui.selectable_value(&mut app.filter_rating, r, if r == 0 { "All".into() } else { format!("≥ {r}★") });
                }
            });
        egui::ComboBox::from_id_salt("flabel").width(70.0).selected_text(app.filter_label.name().unwrap_or("Any label")).show_ui(ui, |ui| {
            for l in ColorLabel::ALL {
                ui.selectable_value(&mut app.filter_label, l, l.name().unwrap_or("Any label"));
            }
        });
        if ui.small_button("Select all").clicked() {
            for d in app.docs.iter_mut() {
                d.selected = true;
            }
        }
    });
    let visible = app.visible_indices();
    let h = (ui.available_height() - 4.0).max(40.0);
    let tw = h * 1.3;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            for &i in &visible {
                let d = &mut app.docs[i];
                if d.thumb.is_none() && !d.thumb_requested {
                    d.thumb_requested = true;
                    app.loader.thumb(d.id, d.path.clone());
                }
                let (rect, resp) = ui.allocate_exact_size(vec2(tw, h), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let p = ui.painter_at(rect);
                let current = app.current == Some(i);
                p.rect_filled(
                    rect,
                    3.0,
                    if current {
                        crate::theme::pal().button_hover
                    } else if d.selected {
                        crate::theme::pal().divider
                    } else {
                        crate::theme::pal().bar
                    },
                );
                let inner = rect.shrink2(vec2(4.0, 14.0)).translate(vec2(0.0, -4.0));
                if let Some(t) = &d.thumb {
                    let s = t.size_vec2();
                    let f = (inner.width() / s.x).min(inner.height() / s.y);
                    let r = egui::Rect::from_center_size(inner.center(), s * f);
                    p.image(t.id(), r, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    p.text(inner.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(14.0), Color32::GRAY);
                }
                let stars: String = (0..5).map(|k| if k < d.rating { '★' } else { '·' }).collect();
                p.text(
                    pos2(rect.left() + 4.0, rect.bottom() - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    stars,
                    egui::FontId::proportional(10.0),
                    Color32::from_rgb(230, 200, 90),
                );
                if d.label != ColorLabel::None {
                    p.rect_filled(egui::Rect::from_min_size(pos2(rect.right() - 12.0, rect.bottom() - 10.0), vec2(8.0, 6.0)), 1.0, d.label.color());
                }
                if d.dirty || d.settings != darkslide_core::DevelopSettings::defaults_for(d.is_raw()) && d.engine.is_some() {
                    p.text(
                        pos2(rect.right() - 4.0, rect.top() + 2.0),
                        egui::Align2::RIGHT_TOP,
                        ic::PENCIL_SIMPLE,
                        egui::FontId::proportional(11.0),
                        Color32::LIGHT_GRAY,
                    );
                }
                if current {
                    p.rect_stroke(rect, 3.0, Stroke::new(1.5, crate::widgets::ACCENT), StrokeKind::Inside);
                } else {
                    p.rect_stroke(rect, 3.0, Stroke::new(1.0, crate::theme::pal().divider), StrokeKind::Inside);
                }
                let resp = resp.on_hover_text(d.name());
                if resp.clicked() {
                    clicked = Some((i, ui.input(|i| i.modifiers)));
                }
            }
        });
    });
    if let Some((i, m)) = clicked {
        if m.command {
            app.docs[i].selected = !app.docs[i].selected;
        } else if m.shift {
            if let Some(c) = app.current {
                let (a, b) = (c.min(i), c.max(i));
                for k in a..=b {
                    app.docs[k].selected = true;
                }
            }
        } else {
            app.select(i);
        }
    }
}
