//! Virtual StreamDeck — a grid of buttons that fire keyboard shortcuts.
//!
//! Designed around *global* hotkeys (Discord mute, ShadowPlay, OBS
//! F13–F24 bindings, Win+Shift+S): those fire no matter which window has
//! focus, so pressing a button in the overlay reaches the target app.

use crate::theme;
use rsoverlay_input::{InputInjector, Shortcut};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroEntry {
    pub label: String,
    pub keys: String,
}

#[derive(Default)]
pub struct MacroDeck {
    pub entries: Vec<MacroEntry>,
    editing: bool,
    /// Inline feedback line ("sent Discord mute" / injector error),
    /// cleared after [`STATUS_TTL`].
    status: Option<(String, bool, Instant)>,
}

const STATUS_TTL: Duration = Duration::from_secs(3);
const COLS: usize = 2;
const MAX_ENTRIES: usize = 24;

impl MacroDeck {
    pub fn card(
        &mut self,
        ui: &mut egui::Ui,
        injector: Option<&InputInjector>,
        accent: egui::Color32,
    ) {
        if self
            .status
            .as_ref()
            .is_some_and(|(_, _, t)| t.elapsed() > STATUS_TTL)
        {
            self.status = None;
        }
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(crate::icons::icon(crate::icons::PULSE, 18.0, accent));
                ui.add_space(6.0);
                ui.label(theme::caption("Macros"));
                crate::widgets::popout_button(ui, "Macros", accent);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.editing { "Done" } else { "Edit" };
                    let text = egui::RichText::new(label)
                        .color(if self.editing {
                            accent
                        } else {
                            theme::FG_MUTED
                        })
                        .size(theme::SIZE_CAPTION)
                        .extra_letter_spacing(1.0);
                    if ui.button(text).clicked() {
                        self.editing = !self.editing;
                    }
                });
            });
            ui.add_space(6.0);
            if self.editing {
                self.edit_list(ui, accent);
            } else {
                self.grid(ui, injector, accent);
            }
            if let Some((msg, ok, _)) = &self.status {
                ui.add_space(4.0);
                let color = if *ok { theme::FG_MUTED } else { theme::DANGER };
                ui.label(
                    egui::RichText::new(msg)
                        .size(theme::SIZE_CAPTION)
                        .color(color),
                );
            }
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, injector: Option<&InputInjector>, accent: egui::Color32) {
        if self.entries.is_empty() {
            ui.label(theme::body_muted("no macros — hit Edit to add one"));
            return;
        }
        let gap = ui.spacing().item_spacing.x;
        let w = (ui.available_width() - gap * (COLS as f32 - 1.0)) / COLS as f32;
        let mut fired = None;
        for row in self.entries.chunks(COLS) {
            ui.horizontal(|ui| {
                for e in row {
                    let parsed = e.keys.parse::<Shortcut>();
                    let enabled = injector.is_some() && parsed.is_ok();
                    let btn = egui::Button::new(egui::RichText::new(&e.label).color(if enabled {
                        theme::FG_PRIMARY
                    } else {
                        theme::FG_MUTED
                    }))
                    .min_size(egui::vec2(w, 34.0))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        if enabled {
                            accent.gamma_multiply(0.5)
                        } else {
                            egui::Color32::from_white_alpha(0x18)
                        },
                    ));
                    let hover = hover_text(&parsed, injector, &e.keys);
                    let resp = ui.add_enabled(enabled, btn);
                    let resp = if enabled {
                        resp.on_hover_text(hover)
                    } else {
                        resp.on_disabled_hover_text(hover)
                    };
                    if resp.clicked() {
                        if let (Ok(s), Some(inj)) = (parsed, injector) {
                            fired = Some((e.label.clone(), inj.send_shortcut(&s)));
                        }
                    }
                }
            });
        }
        if let Some((label, result)) = fired {
            self.status = Some(match result {
                Ok(()) => (format!("sent {label}"), true, Instant::now()),
                Err(e) => (format!("{label}: {e:#}"), false, Instant::now()),
            });
        }
    }

    fn edit_list(&mut self, ui: &mut egui::Ui, accent: egui::Color32) {
        let mut remove = None;
        for (i, e) in self.entries.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut e.label)
                        .desired_width(88.0)
                        .hint_text("label"),
                );
                let err = e.keys.parse::<Shortcut>().err();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut e.keys)
                        .desired_width(110.0)
                        .hint_text("ctrl+shift+m")
                        .text_color(if err.is_some() {
                            theme::DANGER
                        } else {
                            theme::FG_PRIMARY
                        }),
                );
                if let Some(err) = err {
                    resp.on_hover_text(err.to_string());
                }
                if ui
                    .button(egui::RichText::new("×").color(theme::FG_MUTED))
                    .on_hover_text("Remove")
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.entries.remove(i);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can_add = self.entries.len() < MAX_ENTRIES;
            if ui
                .add_enabled(
                    can_add,
                    egui::Button::new(egui::RichText::new("+ add").color(accent)),
                )
                .clicked()
            {
                self.entries.push(MacroEntry {
                    label: "New".into(),
                    keys: "f13".into(),
                });
            }
            ui.label(
                egui::RichText::new("tip: bind OBS scenes to f13–f24")
                    .size(theme::SIZE_CAPTION)
                    .color(theme::FG_MUTED),
            );
        });
    }
}

fn hover_text(
    parsed: &Result<Shortcut, rsoverlay_input::shortcut::ParseError>,
    injector: Option<&InputInjector>,
    keys: &str,
) -> String {
    match (parsed, injector) {
        (Err(err), _) => format!("{keys}: {err}"),
        (Ok(_), None) => "input injection unavailable in this mode".into(),
        (Ok(_), Some(_)) => keys.to_string(),
    }
}
