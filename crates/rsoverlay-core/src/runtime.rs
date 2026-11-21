//! Runtime mode — one source of truth consumed by renderer, input, and app.

/// Where the overlay surface lives this session.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Submit textures to OpenVR; requires SteamVR + HMD.
    Vr,
    /// Present textures into a desktop window for iterative development.
    /// No HMD required.
    Preview,
    /// Render the overlay inside a SteamVR-style 3D preview with a
    /// chaperone-grid floor, 100° HFOV camera, and the overlay quad
    /// placed where a wrist-mounted overlay would sit. Mouse drag
    /// looks around; scroll dollies. No HMD required.
    VrPreview,
}

impl RuntimeMode {
    pub fn is_preview(self) -> bool {
        matches!(self, Self::Preview | Self::VrPreview)
    }

    pub fn is_vr(self) -> bool {
        matches!(self, Self::Vr)
    }
}
