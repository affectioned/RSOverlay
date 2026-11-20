//! Panel = a logical overlay surface. One panel maps to one OpenVR overlay
//! in VR mode, or one window region in Preview mode.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Stable identifier for a panel across config reloads.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PanelId(pub u64);

/// What a panel is showing. Keeps rendering logic ignorant of source.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PanelKind {
    /// Solid-color test surface. Used by preview smoke tests.
    SolidColor { rgba: [u8; 4] },
    /// Full monitor mirror. Index is the Windows DXGI output index.
    Desktop { monitor_index: u32 },
    /// Single window mirror by HWND.
    Window { hwnd: u64 },
    /// egui-driven UI panel.
    Ui { view: String },
}

/// Where the panel is anchored in world/HMD/controller space.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Anchor {
    /// Fixed in standing-universe room space.
    World,
    /// Rigidly attached to the HMD (always in view).
    Hmd,
    /// Attached to the left or right wrist.
    LeftWrist,
    RightWrist,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Panel {
    pub id: PanelId,
    pub kind: PanelKind,
    pub anchor: Anchor,
    /// Meters. World-space position relative to anchor.
    #[serde(with = "vec3_serde")]
    pub position: Vec3,
    /// Rotation relative to anchor.
    #[serde(with = "quat_serde")]
    pub rotation: Quat,
    /// Physical width in meters. Height is derived from aspect.
    pub width_meters: f32,
    /// 0.0..=1.0.
    pub opacity: f32,
    pub visible: bool,
}

impl Panel {
    pub fn solid(id: u64, rgba: [u8; 4]) -> Self {
        Self {
            id: PanelId(id),
            kind: PanelKind::SolidColor { rgba },
            anchor: Anchor::World,
            position: Vec3::new(0.0, 1.5, -2.0),
            rotation: Quat::IDENTITY,
            width_meters: 1.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

mod vec3_serde {
    use glam::Vec3;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec3, s: S) -> Result<S::Ok, S::Error> {
        [v.x, v.y, v.z].serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
        let [x, y, z] = <[f32; 3]>::deserialize(d)?;
        Ok(Vec3::new(x, y, z))
    }
}

mod quat_serde {
    use glam::Quat;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(q: &Quat, s: S) -> Result<S::Ok, S::Error> {
        [q.x, q.y, q.z, q.w].serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Quat, D::Error> {
        let [x, y, z, w] = <[f32; 4]>::deserialize(d)?;
        Ok(Quat::from_xyzw(x, y, z, w))
    }
}
