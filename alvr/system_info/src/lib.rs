#[cfg(target_os = "android")]
pub mod android;

#[cfg(target_os = "android")]
pub use android::*;

use alvr_common::settings_schema::SettingsSchema;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub const PACKAGE_NAME_STORE: &str = "alvr.client";
pub const PACKAGE_NAME_GITHUB_DEV: &str = "alvr.client.dev";
pub const PACKAGE_NAME_GITHUB_STABLE: &str = "alvr.client.stable";

pub const VRX_VISIBLE_EYE_RESOLUTION: alvr_common::glam::UVec2 =
    alvr_common::glam::UVec2::new(2280, 2280);
pub const VRX_ENCODED_EYE_RESOLUTION: alvr_common::glam::UVec2 =
    alvr_common::glam::UVec2::new(2304, 2304);

/// Scale runtime-recommended lobby dimensions without exceeding either runtime limit.
pub fn lobby_resolution(
    recommended: alvr_common::glam::UVec2,
    maximum: alvr_common::glam::UVec2,
    scale: f32,
) -> alvr_common::glam::UVec2 {
    (recommended.as_vec2() * scale).as_uvec2().min(maximum)
}

// Platform of the device. It is used to match the VR runtime and enable features conditionally.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Platform {
    Quest1,
    Quest2,
    Quest3,
    Quest3S,
    QuestPro,
    QuestUnknown,
    PicoNeo3,
    Pico4,
    Pico4Pro,
    Pico4Enterprise,
    Pico4Ultra,
    PicoG3,
    PicoUnknown,
    Focus3,
    FocusVision,
    XRElite,
    ViveUnknown,
    ThinkRealityVRX,
    Yvr,
    PlayForDreamMR,
    SamsungGalaxyXR,
    AndroidUnknown,
    VisionOSHeadset,
    WindowsPc,
    LinuxPc,
    Macos,
    Unknown,
}

impl Platform {
    pub const fn is_quest(&self) -> bool {
        matches!(
            self,
            Self::Quest1
                | Self::Quest2
                | Self::Quest3
                | Self::Quest3S
                | Self::QuestPro
                | Self::QuestUnknown
        )
    }

    pub const fn is_pico(&self) -> bool {
        matches!(
            self,
            Self::PicoG3
                | Self::PicoNeo3
                | Self::Pico4
                | Self::Pico4Pro
                | Self::Pico4Enterprise
                | Self::Pico4Ultra
                | Self::PicoUnknown
        )
    }

    pub const fn is_vive(&self) -> bool {
        matches!(
            self,
            Self::Focus3 | Self::FocusVision | Self::XRElite | Self::ViveUnknown
        )
    }

    pub const fn is_yvr(&self) -> bool {
        matches!(self, Self::Yvr | Self::PlayForDreamMR)
    }
}

impl Display for Platform {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Quest1 => "Quest 1",
            Self::Quest2 => "Quest 2",
            Self::Quest3 => "Quest 3",
            Self::Quest3S => "Quest 3S",
            Self::QuestPro => "Quest Pro",
            Self::QuestUnknown => "Quest (unknown)",
            Self::PicoNeo3 => "Pico Neo 3",
            Self::Pico4 => "Pico 4",
            Self::Pico4Pro => "Pico 4 Pro",
            Self::Pico4Enterprise => "Pico 4 Enterprise",
            Self::Pico4Ultra => "Pico 4 Ultra",
            Self::PicoG3 => "Pico G3",
            Self::PicoUnknown => "Pico (unknown)",
            Self::Focus3 => "VIVE Focus 3",
            Self::FocusVision => "VIVE Focus Vision",
            Self::XRElite => "VIVE XR Elite",
            Self::ViveUnknown => "HTC VIVE (unknown)",
            Self::ThinkRealityVRX => "Lenovo ThinkReality VRX",
            Self::Yvr => "YVR",
            Self::PlayForDreamMR => "Play For Dream MR",
            Self::SamsungGalaxyXR => "Samsung Galaxy XR",
            Self::AndroidUnknown => "Android (unknown)",
            Self::VisionOSHeadset => "visionOS Headset",
            Self::WindowsPc => "Windows PC",
            Self::LinuxPc => "Linux PC",
            Self::Macos => "macOS",
            Self::Unknown => "Unknown",
        };
        write!(f, "{name}")
    }
}

#[cfg_attr(not(target_os = "android"), expect(unused_variables))]
pub fn platform(runtime_name: Option<String>, runtime_version: Option<u64>) -> Platform {
    #[cfg(target_os = "android")]
    {
        let manufacturer = android::manufacturer_name();
        let model = android::model_name();
        let device = android::device_name();
        let product = android::product_name();

        // TODO: Better Android XR heuristic
        // (Maybe check runtime json for /system/lib64/libopenxr.google.so?)

        alvr_common::info!(
            "manufacturer: {manufacturer}, model: {model}, device: {device}, product: {product}, \
            runtime_name: {runtime_name:?}, runtime_version: {runtime_version:?}",
        );

        classify_android_platform(&manufacturer, &model, &device, &product)
    }
    #[cfg(not(target_os = "android"))]
    {
        match std::env::consts::OS {
            "visionos" => Platform::VisionOSHeadset,
            "windows" => Platform::WindowsPc,
            "linux" => Platform::LinuxPc,
            "macos" => Platform::Macos,
            _ => Platform::Unknown,
        }
    }
}

#[cfg(any(target_os = "android", test))]
fn classify_android_platform(
    manufacturer: &str,
    model: &str,
    device: &str,
    product: &str,
) -> Platform {
    match (manufacturer, model, device, product) {
        ("Oculus", _, "monterey", _) => Platform::Quest1,
        ("Oculus", _, "hollywood", _) => Platform::Quest2,
        ("Oculus", _, "eureka", _) => Platform::Quest3,
        ("Oculus", _, "panther", _) => Platform::Quest3S,
        ("Oculus", _, "seacliff", _) => Platform::QuestPro,
        ("Oculus", _, _, _) => Platform::QuestUnknown,
        ("Pico", "Pico Neo 3" | "Pico Neo3 Link", _, _) => Platform::PicoNeo3,
        ("Pico", _, _, "PICO 4 Pro") => Platform::Pico4Pro,
        ("Pico", _, _, "PICO 4 Enterprise") => Platform::Pico4Enterprise,
        ("Pico", _, _, "PICO 4") => Platform::Pico4,
        ("Pico", _, _, "PICO 4 Ultra") => Platform::Pico4Ultra,
        ("Pico", _, _, "PICO G3") => Platform::PicoG3,
        ("Pico", _, _, _) => Platform::PicoUnknown,
        ("HTC", "VIVE Focus 3", _, _) => Platform::Focus3,
        ("HTC", "VIVE Focus Vision", _, _) => Platform::FocusVision,
        ("HTC", "VIVE XR Series", _, _) => Platform::XRElite,
        ("HTC", _, _, _) => Platform::ViveUnknown,
        ("Lenovo", "VRX", _, _) => Platform::ThinkRealityVRX,
        ("YVR", _, _, _) => Platform::Yvr,
        ("Play For Dream", _, _, _) => Platform::PlayForDreamMR,
        ("samsung", _, "xrvst2", _) => Platform::SamsungGalaxyXR,
        _ => Platform::AndroidUnknown,
    }
}

#[cfg(not(target_os = "android"))]
pub fn local_ip() -> std::net::IpAddr {
    use std::net::{IpAddr, Ipv4Addr};

    local_ip_address::local_ip().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

#[derive(SettingsSchema, Serialize, Deserialize, Clone)]
pub enum ClientFlavor {
    Store,
    Github,
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::{Platform, classify_android_platform};

    #[test]
    fn identifies_think_reality_vrx() {
        assert_eq!(
            classify_android_platform("Lenovo", "VRX", "kona", "kona"),
            Platform::ThinkRealityVRX
        );
    }

    #[test]
    fn does_not_misidentify_other_lenovo_devices() {
        assert_eq!(
            classify_android_platform("Lenovo", "ThinkPad", "kona", "kona"),
            Platform::AndroidUnknown
        );
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use alvr_common::glam::UVec2;
    #[test]
    fn quest3_supersampling_respects_runtime_limits() {
        let recommended = UVec2::new(2064, 2208);
        assert_eq!(
            lobby_resolution(recommended, UVec2::splat(4096), 1.5),
            UVec2::new(3096, 3312)
        );
        assert_eq!(
            lobby_resolution(recommended, UVec2::new(4000, 4100), 2.0),
            UVec2::new(4000, 4100)
        );
    }
    #[test]
    fn vrx_padding_is_twelve_pixels_per_edge() {
        assert_eq!(
            (VRX_ENCODED_EYE_RESOLUTION - VRX_VISIBLE_EYE_RESOLUTION) / 2,
            UVec2::splat(12)
        );
        assert_eq!(VRX_ENCODED_EYE_RESOLUTION.x * 2, 4608);
    }
}
