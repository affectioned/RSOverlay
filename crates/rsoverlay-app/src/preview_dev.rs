//! Desktop dev harness: every VR surface, switchable live in the desktop
//! window at its real size in points, with demo-data controls — so UI
//! work can be checked without a headset or relaunching with flags.
//!
//! The bar is hidden for `--screenshot` runs so store / doc captures
//! stay clean; the CLI flags (`--wrist`, `--widgets`, `--vr-keyboard`,
//! `--dock`) still pick the starting surface.

use crate::preview::TestHooks;
use rsoverlay_ui::laser::Hand;
use rsoverlay_ui::views::DevView;
use rsoverlay_ui::widgets::WidgetKind;
use rsoverlay_ui::window_panel::{self, Attach};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Dashboard,
    VrDashboard,
    Wrist,
    VrKeyboard,
    Widgets,
    WindowPanel,
    Toasts,
}

impl Surface {
    const ALL: [Surface; 7] =
        [Self::Dashboard, Self::VrDashboard, Self::Wrist, Self::VrKeyboard, Self::Widgets, Self::WindowPanel, Self::Toasts];

    fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Desktop",
            Self::VrDashboard => "VR dashboard",
            Self::Wrist => "Wrist",
            Self::VrKeyboard => "VR keyboard",
            Self::Widgets => "Widgets",
            Self::WindowPanel => "Window panel",
            Self::Toasts => "Toasts",
        }
    }

    /// Width in egui points the surface has in VR (texture / ppp), so the
    /// preview shows real proportions. `None` = fills the window.
    fn vr_width_pt(self) -> Option<f32> {
        match self {
            Self::Dashboard => None,
            Self::VrDashboard => Some(1024.0),
            Self::Wrist | Self::Widgets | Self::Toasts => Some(600.0),
            Self::VrKeyboard => Some(1024.0),
            Self::WindowPanel => Some(1280.0),
        }
    }
}

const WIDGET_KINDS: [WidgetKind; 7] = [
    WidgetKind::NowPlaying,
    WidgetKind::System,
    WidgetKind::Volume,
    WidgetKind::Battery,
    WidgetKind::Macros,
    WidgetKind::Chat,
    WidgetKind::Notifications,
];

pub struct DevHarness {
    pub surface: Surface,
    show_bar: bool,
    widgets: Vec<WidgetKind>,
    widget_active: usize,
    demo_batteries: bool,
    win_attach: Attach,
    win_linked: bool,
    win_pinned: bool,
    win_opacity: f32,
    /// What the VR keyboard typed (it types into RSOverlay, not Windows).
    typed: String,
    icons: crate::device_icons::DeviceIcons,
    /// Last applied "SteamVR device icons" setting (demo devices follow it live).
    icons_on: bool,
}

impl DevHarness {
    pub fn new(test: &TestHooks) -> Self {
        let widgets: Vec<WidgetKind> = test
            .widgets
            .as_deref()
            .map(|s| s.split(',').filter_map(WidgetKind::from_slug).collect())
            .unwrap_or_default();
        let surface = if test.vr_keyboard {
            Surface::VrKeyboard
        } else if !widgets.is_empty() {
            Surface::Widgets
        } else if test.wrist {
            Surface::Wrist
        } else if test.dock {
            Surface::VrDashboard
        } else {
            Surface::Dashboard
        };
        Self {
            surface,
            // RSO_DEV_BAR=1 keeps it in screenshots (checking the bar itself).
            show_bar: test.screenshot.is_none() || std::env::var_os("RSO_DEV_BAR").is_some(),
            widgets: if widgets.is_empty() { vec![WidgetKind::System, WidgetKind::Volume] } else { widgets },
            widget_active: 0,
            demo_batteries: test.demo_batteries,
            win_attach: Attach::World,
            win_linked: true,
            win_pinned: false,
            win_opacity: 1.0,
            typed: String::new(),
            icons: Default::default(),
            icons_on: true,
        }
    }

    /// Draw the dev bar (unless screenshotting) and the chosen surface.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        dev: &mut DevView,
        keyboard: &mut rsoverlay_ui::vrkeyboard::VrKeyboard,
        injector: &rsoverlay_input::InputInjector,
    ) {
        // Demo devices follow the "SteamVR device icons" toggle live.
        if self.demo_batteries && dev.steamvr_icons != self.icons_on {
            self.icons_on = dev.steamvr_icons;
            dev.set_batteries(demo_batteries(0.82, &mut self.icons, self.icons_on));
        }
        if self.show_bar {
            self.bar(ctx, dev);
        }
        dev.dock_layout = self.surface == Surface::VrDashboard;
        self.frame_to_vr_width(ctx);
        match self.surface {
            Surface::Dashboard => dev.ui_with(ctx, Some(injector)),
            Surface::VrDashboard => {
                dev.ui_with(ctx, Some(injector));
                // The VR shell handles these in a headset; here, Dock just
                // highlights and ✕ goes back to the desktop dashboard.
                if let Some(a) = dev.dash_dock.take() {
                    dev.dash_attach = a;
                }
                if std::mem::take(&mut dev.dash_close) {
                    self.surface = Surface::Dashboard;
                }
            }
            Surface::Wrist => {
                let _ = dev.wrist_ui(ctx);
            }
            Surface::VrKeyboard => {
                // The mouse plays the laser: hover = aim, left button = trigger.
                let (pos, down, t) = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_down(), i.time));
                let hands = [(Hand::Right, pos, if down { 1.0 } else { 0.0 })];
                let kf = keyboard.frame(ctx, &hands, t, dev.accent, true);
                for e in kf.text {
                    match e {
                        egui::Event::Text(s) => self.typed.push_str(&s),
                        egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                            self.typed.pop();
                        }
                        _ => {}
                    }
                }
                if let Some(msg) = kf.chat {
                    self.typed = format!("[VRChat] {msg}");
                }
                if kf.close {
                    self.surface = Surface::Dashboard;
                }
            }
            Surface::Widgets => {
                self.widget_active = self.widget_active.min(self.widgets.len().saturating_sub(1));
                let wa = dev.widget_ui(ctx, &self.widgets, self.widget_active, self.win_attach, self.win_linked, Some(self.win_pinned), Some(injector));
                if let Some(i) = wa.select {
                    self.widget_active = i;
                }
                if wa.unlink {
                    self.win_linked = false;
                }
                self.win_pinned ^= wa.toggle_pin;
                if let Some(a) = wa.attach {
                    self.win_attach = a;
                }
                if wa.close {
                    self.surface = Surface::Dashboard;
                }
            }
            Surface::WindowPanel => {
                let act = window_panel::show(ctx, "Example window — capture placeholder", None, self.win_attach, false, None, self.win_linked, self.win_opacity, Some(self.win_pinned), true, false, dev.accent);
                if let Some(a) = act.attach {
                    self.win_attach = a;
                }
                if act.unlink {
                    self.win_linked = false;
                }
                self.win_pinned ^= act.toggle_pin;
                if let Some(o) = act.opacity {
                    self.win_opacity = o;
                }
                if act.close {
                    self.surface = Surface::Dashboard;
                }
            }
            Surface::Toasts => {
                dev.toasts_ui(ctx);
                if !dev.has_toasts() {
                    egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                        ui.centered_and_justified(|ui| ui.label("No toasts — use the Notify buttons above."));
                    });
                }
            }
        }
    }

    /// Pad the window so a VR surface gets its real VR width in points
    /// (or the whole width if the window is narrower), and — for the VR
    /// keyboard, which fills its panel — its real aspect too.
    fn frame_to_vr_width(&self, ctx: &egui::Context) {
        let Some(w) = self.surface.vr_width_pt() else { return };
        let frame = egui::Frame::none().fill(egui::Color32::from_gray(18));
        let avail = ctx.available_rect().width();
        let pad = ((avail - w) / 2.0).floor();
        if pad >= 8.0 {
            egui::SidePanel::left("dev-pad-l").exact_width(pad).resizable(false).show_separator_line(false).frame(frame).show(ctx, |_| {});
            egui::SidePanel::right("dev-pad-r").exact_width(pad).resizable(false).show_separator_line(false).frame(frame).show(ctx, |_| {});
        }
        if self.surface == Surface::VrKeyboard {
            let r = ctx.available_rect();
            let spare = r.height() - r.width() * 400.0 / 1024.0;
            if spare > 8.0 {
                egui::TopBottomPanel::bottom("dev-pad-b").exact_height(spare).resizable(false).show_separator_line(false).frame(frame).show(ctx, |_| {});
            }
        }
    }

    fn bar(&mut self, ctx: &egui::Context, dev: &mut DevView) {
        egui::TopBottomPanel::top("dev-bar")
            .frame(egui::Frame::none().fill(egui::Color32::from_gray(22)).inner_margin(egui::Margin::symmetric(12.0, 8.0)))
            .show(ctx, |ui| {
                ui.visuals_mut().selection.bg_fill = dev.accent;
                ui.visuals_mut().selection.stroke.color = egui::Color32::BLACK;
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("DEV").strong().color(dev.accent));
                    for s in Surface::ALL {
                        if ui.selectable_label(self.surface == s, s.label()).clicked() {
                            self.surface = s;
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut self.demo_batteries, "Demo batteries").changed() {
                        dev.set_batteries(if self.demo_batteries { demo_batteries(0.82, &mut self.icons, dev.steamvr_icons) } else { Vec::new() });
                    }
                    if ui.button("Low battery").on_hover_text("Drop the headset below the warning threshold (toast + chime)").clicked() {
                        self.demo_batteries = true;
                        dev.set_batteries(demo_batteries(0.82, &mut self.icons, dev.steamvr_icons));
                        dev.set_batteries(demo_batteries(0.05, &mut self.icons, dev.steamvr_icons));
                    }
                    ui.separator();
                    for (label, path) in [("Notify", "default"), ("Warning", "warning"), ("Error", "error"), ("Silent", "none")] {
                        if ui.button(label).on_hover_text(format!("Push a test notification (audioPath = \"{path}\")")).clicked() {
                            dev.push_notification(test_notification(label, path));
                        }
                    }
                    if self.surface == Surface::Widgets {
                        ui.separator();
                        for k in WIDGET_KINDS {
                            let mut on = self.widgets.contains(&k);
                            if ui.checkbox(&mut on, k.label()).changed() {
                                if on {
                                    self.widgets.push(k);
                                } else if self.widgets.len() > 1 {
                                    self.widgets.retain(|w| *w != k);
                                }
                            }
                        }
                    }
                    if matches!(self.surface, Surface::Widgets | Surface::WindowPanel) {
                        ui.separator();
                        ui.checkbox(&mut self.win_linked, "Linked (shows Unlink)");
                    }
                    if self.surface == Surface::VrKeyboard {
                        ui.separator();
                        ui.label(format!("Typed: {}", if self.typed.is_empty() { "—" } else { &self.typed }));
                        if ui.small_button("Clear").clicked() {
                            self.typed.clear();
                        }
                    }
                });
            });
    }
}

fn demo_batteries(hmd: f32, icons: &mut crate::device_icons::DeviceIcons, use_icons: bool) -> Vec<rsoverlay_ui::battery::BatteryReading> {
    let mut r = |id, label: &str, fraction, charging| rsoverlay_ui::battery::BatteryReading {
        id,
        label: label.into(),
        fraction,
        charging,
        icon: if use_icons { icons.for_label(label) } else { None },
    };
    vec![
        r(0, "HMD", hmd, false),
        r(1, "Left hand", 0.64, false),
        r(2, "Right hand", 0.47, true),
        r(3, "Tracker · VIVE Tracker 3.0", 0.71, false),
    ]
}

fn test_notification(kind: &str, audio_path: &str) -> rsoverlay_notify::Notification {
    rsoverlay_notify::Notification {
        title: format!("{kind} test"),
        content: "Sent from the dev bar.".into(),
        audio_path: audio_path.into(),
        source_app: "RSOverlay dev".into(),
        ..Default::default()
    }
}
