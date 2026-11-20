//! Runtime mode — one source of truth consumed by renderer, input, and app.

/// Where the overlay surface lives this session.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Submit textures to OpenVR; requires SteamVR + HMD.
    Vr,
    /// Present textures into a desktop window for iterative development.
    /// No HMD required. Feature parity with Vr for every subsystem except
    /// the final texture submission step.
    Preview,
}

impl RuntimeMode {
    pub fn is_preview(self) -> bool {
        matches!(self, Self::Preview)
    }

    pub fn is_vr(self) -> bool {
        matches!(self, Self::Vr)
    }
}
