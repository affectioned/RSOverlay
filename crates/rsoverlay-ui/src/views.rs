//! Built-in egui views used by the default panel set.

/// State for the dev/diagnostics view shown in the P0 preview overlay.
pub struct DevView {
    pub frame: u64,
    pub last_fps: f32,
    pub message: String,
}

impl Default for DevView {
    fn default() -> Self {
        Self {
            frame: 0,
            last_fps: 0.0,
            message: "RSOverlay preview".to_string(),
        }
    }
}

impl DevView {
    pub fn ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.heading("RSOverlay");
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&self.message).weak());
                ui.add_space(24.0);
                ui.separator();
                ui.add_space(8.0);
                egui::Grid::new("dev-grid").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    ui.label("frame");
                    ui.monospace(self.frame.to_string());
                    ui.end_row();
                    ui.label("fps");
                    ui.monospace(format!("{:.1}", self.last_fps));
                    ui.end_row();
                    ui.label("mode");
                    ui.monospace("preview");
                    ui.end_row();
                });
                ui.add_space(24.0);
                if ui.button("send test toast").clicked() {
                    self.message = format!("toast fired at frame {}", self.frame);
                }
            });
        });
    }
}
