//! Layouts page: 8 save slots for the whole VR arrangement (XSO-style).
//! Tap a filled slot to load it; save the current arrangement into any
//! slot; star one to load it automatically when VR starts.

use crate::theme;
use crate::window_panel::pill;
use rsoverlay_core::layouts::{Layouts, SLOTS};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayoutAction {
    Load(usize),
    Save(usize),
    Clear(usize),
    ToggleFavorite(usize),
}

pub fn page(ui: &mut egui::Ui, layouts: &Layouts, vr: bool, accent: egui::Color32) -> Option<LayoutAction> {
    let mut out = None;
    ui.label(theme::body_muted(if vr {
        "Save every floating window, widget and the keyboard — position, size and dock — into a slot. Tap a slot to load it; ★ loads it when VR starts."
    } else {
        "Layouts are saved and loaded from inside VR. Slots saved there show up here."
    }));
    ui.add_space(theme::GUTTER);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let gap = theme::GUTTER;
    let w = (ui.available_width() - gap) / 2.0;
    for row in 0..SLOTS / 2 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for col in 0..2 {
                let i = row * 2 + col;
                let slot = layouts.slot(i);
                let fav = layouts.favorite == Some(i);
                // Top-down inside each tile (the row itself is horizontal).
                ui.allocate_ui_with_layout(egui::vec2(w, 118.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    theme::card().show(ui, |ui| {
                        ui.set_width(w - 2.0 * theme::PAD_CARD_H - 2.0);
                        ui.horizontal(|ui| {
                            let title = slot.map_or_else(|| format!("Slot {}", i + 1), |l| {
                                if l.name.is_empty() { format!("Slot {}", i + 1) } else { l.name.clone() }
                            });
                            ui.label(theme::card_title(&title));
                            if fav {
                                ui.label(egui::RichText::new("★ startup").size(theme::SIZE_CAPTION + 1.0).color(accent));
                            }
                        });
                        match slot {
                            Some(l) => {
                                let wins = l.panels.iter().filter(|p| p.kind != "widgets").count();
                                let wids = l.panels.iter().filter(|p| p.kind == "widgets").count();
                                let ago = crate::views::relative_time_secs(now.saturating_sub(l.saved_unix));
                                let kb = if l.keyboard.is_some() { " · keyboard" } else { "" };
                                ui.add(egui::Label::new(theme::body_muted(&format!("{wins} windows · {wids} widgets{kb} · {ago} ago"))).truncate(true));
                            }
                            None => {
                                ui.label(theme::body_muted("empty"));
                            }
                        }
                        ui.horizontal(|ui| {
                            if slot.is_some() && vr && pill(ui, "Load", false, accent).clicked() {
                                out = Some(LayoutAction::Load(i));
                            }
                            if vr && pill(ui, "Save here", false, accent).clicked() {
                                out = Some(LayoutAction::Save(i));
                            }
                            if slot.is_some() {
                                if pill(ui, if fav { "★" } else { "☆" }, fav, accent).on_hover_text("Load at VR startup").clicked() {
                                    out = Some(LayoutAction::ToggleFavorite(i));
                                }
                                if pill(ui, "Clear", false, theme::DANGER).clicked() {
                                    out = Some(LayoutAction::Clear(i));
                                }
                            }
                        });
                    });
                });
            }
        });
        ui.add_space(gap);
    }
    out
}
