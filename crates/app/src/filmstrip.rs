//! Filmstrip: thumbnails, ratings, labels, selection, filtering.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use egui_phosphor::regular as ic;

use crate::app::App;
use crate::doc::ColorLabel;

pub fn filmstrip(app: &mut App, ui: &mut egui::Ui) {
    let visible = app.visible_indices();
    // the open photo was filtered out: move to the nearest one still shown
    if let Some(c) = app.current
        && !visible.contains(&c)
        && let Some(&n) = visible.iter().min_by_key(|&&i| i.abs_diff(c))
    {
        app.select(n);
    }
    ui.horizontal(|ui| {
        let n_sel = visible.iter().filter(|&&i| app.docs[i].selected).count();
        let total = app.docs.len();
        let mut head = if visible.len() == total { format!("{total} images") } else { format!("{} of {total} shown", visible.len()) };
        if n_sel > 1 {
            head.push_str(&format!(" · {n_sel} selected"));
        }
        ui.label(RichText::new(head).size(12.0).weak());
        ui.separator();
        ui.label(RichText::new(format!("{}  Filter", ic::FUNNEL_SIMPLE)).size(12.0));
        egui::ComboBox::from_id_salt("frating")
            .width(70.0)
            .selected_text(if app.filter_rating == 0 { "All".into() } else { format!("≥ {}★", app.filter_rating) })
            .show_ui(ui, |ui| {
                for r in 0..=5 {
                    ui.selectable_value(&mut app.filter_rating, r, if r == 0 { "All".into() } else { format!("≥ {r}★") });
                }
            });
        egui::ComboBox::from_id_salt("flabel").width(90.0).selected_text(app.filter_label.name().unwrap_or("Any label")).show_ui(ui, |ui| {
            for l in ColorLabel::ALL {
                ui.selectable_value(&mut app.filter_label, l, l.name().unwrap_or("Any label"));
            }
        });
        if ui.button("Select all").clicked() {
            app.select_all_visible();
        }
    });
    let h = (ui.available_height() - 4.0).max(40.0);
    let tw = h * 1.3;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let mut menu: Option<(usize, MenuAct)> = None;
    if visible.is_empty() && !app.docs.is_empty() {
        ui.add_space(h / 2.0 - 10.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No photos match the filter").weak()));
        return;
    }
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
                let pal = crate::theme::pal();
                p.rect_filled(rect, 3.0, if current || d.selected { pal.button_hover } else { pal.bar });
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
                // edited means develop settings, not a rating or label
                if d.loaded && d.settings != darkslide_core::DevelopSettings::defaults_for(d.raw) {
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
                } else if d.selected {
                    p.rect_stroke(rect, 3.0, Stroke::new(1.0, pal.text_dim), StrokeKind::Inside);
                } else {
                    p.rect_stroke(rect, 3.0, Stroke::new(1.0, pal.divider), StrokeKind::Inside);
                }
                let resp = resp.on_hover_text(d.name());
                if resp.clicked() {
                    clicked = Some((i, ui.input(|i| i.modifiers)));
                }
                resp.context_menu(|ui| {
                    ui.menu_button("Rating", |ui| {
                        for r in 0..=5 {
                            let t = if r == 0 { "No rating".to_string() } else { "★".repeat(r) };
                            if ui.button(format!("{t}    {r}")).clicked() {
                                menu = Some((i, MenuAct::Rating(r as i32)));
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Label", |ui| {
                        for (k, l) in [ColorLabel::Red, ColorLabel::Yellow, ColorLabel::Green, ColorLabel::Blue, ColorLabel::Purple].into_iter().enumerate() {
                            let key = if k < 4 { format!("    {}", k + 6) } else { String::new() };
                            if ui.button(RichText::new(format!("{}  {}{key}", ic::CIRCLE, l.name().unwrap_or(""))).color(l.color())).clicked() {
                                menu = Some((i, MenuAct::Label(l)));
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button(format!("{}  Copy settings", ic::COPY)).clicked() {
                        menu = Some((i, MenuAct::Copy));
                        ui.close();
                    }
                    if ui.add_enabled(app.clipboard.is_some(), egui::Button::new(format!("{}  Paste settings", ic::CLIPBOARD))).clicked() {
                        menu = Some((i, MenuAct::Paste));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(format!("{}  Show in folder", ic::FOLDER_OPEN)).clicked() {
                        menu = Some((i, MenuAct::Reveal));
                        ui.close();
                    }
                    if ui.button(format!("{}  Remove from filmstrip", ic::X)).clicked() {
                        menu = Some((i, MenuAct::Remove));
                        ui.close();
                    }
                });
            }
        });
    });
    if let Some((i, act)) = menu {
        // a right-clicked photo outside the selection becomes the selection, as in Lightroom
        if !app.docs[i].selected {
            app.select(i);
        }
        match act {
            MenuAct::Rating(r) => app.set_rating(r),
            MenuAct::Label(l) => app.toggle_label(l),
            MenuAct::Copy => app.copy_settings_of(i, None),
            MenuAct::Paste => app.paste_settings(None),
            MenuAct::Reveal => {
                if let Some(dir) = app.docs[i].path.parent() {
                    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                }
            }
            MenuAct::Remove => app.remove_doc(i),
        }
    }
    if let Some((i, m)) = clicked {
        if m.command {
            app.docs[i].selected = !app.docs[i].selected;
        } else if m.shift {
            if let Some(c) = app.current {
                let (a, b) = (c.min(i), c.max(i));
                for k in visible.into_iter().filter(|k| (a..=b).contains(k)) {
                    app.docs[k].selected = true;
                }
            }
        } else {
            app.select(i);
        }
    }
}

enum MenuAct {
    Rating(i32),
    Label(ColorLabel),
    Copy,
    Paste,
    Reveal,
    Remove,
}
