use super::schema::{
    HigherOrderChoiceOption, HigherOrderChoiceSchema, PresetModifier, PresetSchemaNode,
};
use crate::dashboard::components::presets::schema::PresetModifierOperation;
use settings_schema::ChoiceControlType;
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

fn string_modifier(target_path: &str, value: &str) -> PresetModifier {
    PresetModifier {
        target_path: target_path.into(),
        operation: PresetModifierOperation::Assign(serde_json::Value::String(value.into())),
    }
}
fn num_modifier(target_path: &str, value: &str) -> PresetModifier {
    PresetModifier {
        target_path: target_path.into(),
        operation: PresetModifierOperation::Assign(serde_json::Value::Number(
            serde_json::Number::from_str(value).unwrap(),
        )),
    }
}
fn bool_modifier(target_path: &str, value: bool) -> PresetModifier {
    PresetModifier {
        target_path: target_path.into(),
        operation: PresetModifierOperation::Assign(serde_json::Value::Bool(value)),
    }
}

pub fn resolution_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Resolution".into(),
        strings: [(
            "help".into(),
            "Choosing too high resolution (commonly 'High (width: 5184)') may result in high latency or black screen.".into(),
        )]
        .into_iter()
        .collect(),
        flags: ["steamvr-restart".into()].into_iter().collect(),
        options: [
            ("Very Low (width: 3072)", "1536"),
            ("Low (width: 3712)", "1856"),
            ("Medium (width: 4288)", "2144"),
            ("High (width: 5184)", "2592"),
            ("Ultra (width: 5632)", "2816"),
            ("Extreme (width: 6080)", "3040"),
        ]
        .into_iter()
        .map(|(key, value)| HigherOrderChoiceOption {
            display_name: key.into(),
            modifiers: [
                string_modifier(
                    "session_settings.video.transcoding_view_resolution.variant",
                    "Absolute",
                ),
                num_modifier(
                    "session_settings.video.transcoding_view_resolution.Absolute.width",
                    value,
                ),
                bool_modifier(
                    "session_settings.video.transcoding_view_resolution.Absolute.height.set",
                    false,
                ),
                string_modifier(
                    "session_settings.video.emulated_headset_view_resolution.variant",
                    "Absolute",
                ),
                num_modifier(
                    "session_settings.video.emulated_headset_view_resolution.Absolute.width",
                    value,
                ),
                bool_modifier(
                    "session_settings.video.emulated_headset_view_resolution.Absolute.height.set",
                    false,
                ),
            ]
            .into_iter()
            .collect(),
            content: None,
        })
        .chain([
            ("ThinkReality VRX (native: 2280 x 2280)", "2280"),
            ("ThinkReality VRX (80%: 1824 x 1824)", "1824"),
        ].into_iter().map(|(label, size)| HigherOrderChoiceOption {
            display_name: label.into(),
            modifiers: ["transcoding_view_resolution", "emulated_headset_view_resolution"]
                .into_iter()
                .flat_map(|field| {
                    let prefix = format!("session_settings.video.{field}");
                    [
                        string_modifier(&format!("{prefix}.variant"), "Absolute"),
                        num_modifier(&format!("{prefix}.Absolute.width"), size),
                        bool_modifier(&format!("{prefix}.Absolute.height.set"), true),
                        num_modifier(&format!("{prefix}.Absolute.height.content"), size),
                    ]
                })
                .collect(),
            content: None,
        }))
        .collect(),
        default_option_display_name: "Medium (width: 4288)".into(),
        gui: ChoiceControlType::Dropdown,
    })
}

pub fn framerate_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Preferred framerate".into(),
        strings: HashMap::new(),
        flags: ["steamvr-restart".into()].into_iter().collect(),
        options: [60, 72, 80, 90, 120]
            .into_iter()
            .map(|framerate| HigherOrderChoiceOption {
                display_name: format!("{framerate}Hz"),
                modifiers: [num_modifier(
                    "session_settings.video.preferred_fps",
                    &format!("{:?}", framerate as f32),
                )]
                .into_iter()
                .collect(),
                content: None,
            })
            .collect(),
        default_option_display_name: "72Hz".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

pub fn codec_preset_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Codec preset".into(),
        strings: [
            (
            "notice".into(),
            "AV1 encoding is only supported on RDNA3, Ada Lovelace, Intel ARC or newer GPUs (AMD RX 7xxx+ , NVIDIA RTX 40xx+, Intel ARC)
and on headsets that have XR2 Gen 2 onboard (Quest 3, Pico 4 Ultra).\n
H264 encoding is currently NOT supported on Intel ARC GPUs on Windows."
                .into(),
            ),
        ]
        .into_iter()
        .collect(),
        flags: ["steamvr-restart".into()].into_iter().collect(),
        options: [("H264", "H264"), ("HEVC", "Hevc"), ("AV1", "AV1")]
            .into_iter()
            .map(|(key, val_codec)| HigherOrderChoiceOption {
                display_name: key.into(),
                modifiers: [string_modifier(
                    "session_settings.video.preferred_codec.variant",
                    val_codec,
                )]
                .into_iter()
                .collect(),
                content: None,
            })
            .collect(),
        default_option_display_name: "H264".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

pub fn encoder_preset_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Encoder preset".into(),
        strings: [(
            "help".into(),
            "Selecting a quality too high may result in stuttering or still image!".into(),
        )]
        .into_iter()
        .collect(),
        flags: ["steamvr-restart".into()].into_iter().collect(),
        options: [
            ("Speed", "Speed", "P1"),
            ("Balanced", "Balanced", "P3"),
            ("Quality", "Quality", "P5"),
        ]
        .into_iter()
        .map(|(key, val_amd, val_nv)| HigherOrderChoiceOption {
            display_name: key.into(),
            modifiers: [
                string_modifier(
                    "session_settings.video.encoder_config.nvenc.quality_preset.variant",
                    val_nv,
                ),
                string_modifier(
                    "session_settings.video.encoder_config.quality_preset.variant",
                    val_amd,
                ),
            ]
            .into_iter()
            .collect(),
            content: None,
        })
        .collect(),
        default_option_display_name: "Speed".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

pub fn foveation_preset_schema() -> PresetSchemaNode {
    const PREFIX: &str = "session_settings.video.foveated_encoding";
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Foveation preset".into(),
        strings: [(
            "help".into(),
            "Foveation affects pixelation on the edges of \
            the screen and significantly reduces codec latency. 
It is not recommended to fully disable it, as it may cause \
shutterring and high encode/decode latency!"
                .into(),
        )]
        .into_iter()
        .collect(),
        flags: ["steamvr-restart".into()].into_iter().collect(),
        options: [
            ("Light", [0.80, 0.80], [8.0, 8.0]),
            ("Medium", [0.66, 0.60], [6.0, 6.0]),
            ("High", [0.45, 0.40], [4.0, 5.0]),
        ]
        .into_iter()
        .map(|(key, center_size, edge_ratio)| {
            let [center_size_x, center_size_y] = center_size;
            let [edge_ratio_x, edge_ratio_y] = edge_ratio;

            HigherOrderChoiceOption {
                display_name: key.into(),
                modifiers: [
                    bool_modifier(&format!("{PREFIX}.enabled"), true),
                    num_modifier(
                        &format!("{PREFIX}.content.center_size.content[0]"),
                        &center_size_x.to_string(),
                    ),
                    num_modifier(
                        &format!("{PREFIX}.content.center_size.content[1]"),
                        &center_size_y.to_string(),
                    ),
                    num_modifier(
                        &format!("{PREFIX}.content.edge_ratio.content[0]"),
                        &edge_ratio_x.to_string(),
                    ),
                    num_modifier(
                        &format!("{PREFIX}.content.edge_ratio.content[1]"),
                        &edge_ratio_y.to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
                content: None,
            }
        })
        .collect(),
        default_option_display_name: "High".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

#[cfg(target_os = "linux")]
pub fn game_audio_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Headset speaker".into(),
        strings: HashMap::new(),
        flags: HashSet::new(),
        options: [
            HigherOrderChoiceOption {
                display_name: "Disabled".into(),
                modifiers: vec![bool_modifier(
                    "session_settings.audio.game_audio.enabled",
                    false,
                )],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "Enabled".into(),
                modifiers: vec![bool_modifier(
                    "session_settings.audio.game_audio.enabled",
                    true,
                )],
                content: None,
            },
        ]
        .into_iter()
        .collect(),
        default_option_display_name: "Enabled".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

#[cfg(target_os = "linux")]
pub fn microphone_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Headset microphone".into(),
        strings: HashMap::new(),
        flags: HashSet::new(),
        options: [
            HigherOrderChoiceOption {
                display_name: "Disabled".into(),
                modifiers: vec![bool_modifier(
                    "session_settings.audio.microphone.enabled",
                    false,
                )],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "Enabled".into(),
                modifiers: vec![bool_modifier(
                    "session_settings.audio.microphone.enabled",
                    true,
                )],
                content: None,
            },
        ]
        .into_iter()
        .collect(),
        default_option_display_name: "Enabled".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn game_audio_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Headset speaker".into(),
        strings: [(
            "notice".into(),
            "You can change the default audio device from the system taskbar tray (bottom right)"
                .into(),
        )]
        .into_iter()
        .collect(),
        flags: HashSet::new(),
        options: vec![
            HigherOrderChoiceOption {
                display_name: "Disabled".into(),
                modifiers: vec![bool_modifier(
                    "session_settings.audio.game_audio.enabled",
                    false,
                )],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "System Default".to_owned(),
                modifiers: vec![
                    bool_modifier("session_settings.audio.game_audio.enabled", true),
                    bool_modifier(
                        "session_settings.audio.game_audio.content.device.set",
                        false,
                    ),
                ],
                content: None,
            },
        ]
        .into_iter()
        .collect(),
        default_option_display_name: "System Default".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn microphone_schema() -> PresetSchemaNode {
    let mut microhone_options = vec![HigherOrderChoiceOption {
        display_name: "Disabled".to_owned(),
        modifiers: vec![bool_modifier(
            "session_settings.audio.microphone.enabled",
            false,
        )],
        content: None,
    }];

    if cfg!(windows) {
        for (key, display_name) in [
            ("Automatic", "Automatic"),
            ("VAC", "Virtual Audio Cable"),
            ("VBCable", "VB Cable"),
            ("VoiceMeeter", "VoiceMeeter"),
            ("VoiceMeeterAux", "VoiceMeeter Aux"),
            ("VoiceMeeterVaio3", "VoiceMeeter VAIO3"),
        ] {
            microhone_options.push(HigherOrderChoiceOption {
                display_name: display_name.into(),
                modifiers: vec![
                    bool_modifier("session_settings.audio.microphone.enabled", true),
                    string_modifier(
                        "session_settings.audio.microphone.content.devices.variant",
                        key,
                    ),
                ],
                content: None,
            })
        }
    }

    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Headset microphone".into(),
        strings: HashMap::new(),
        flags: HashSet::new(),
        options: microhone_options.into_iter().collect(),
        default_option_display_name: "Disabled".into(),
        gui: ChoiceControlType::Dropdown,
    })
}

pub fn hand_tracking_interaction_schema() -> PresetSchemaNode {
    const HELP: &str = r"Disabled: hands cannot emulate buttons. Useful for using Joy-Cons or other non-native controllers.
SteamVR Input 2.0: create separate SteamVR devices for hand tracking.
ALVR bindings: use ALVR hand tracking button bindings. Check the wiki for help.
";

    const PREFIX: &str = "session_settings.headset.controllers.content";

    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Hand tracking interaction".into(),
        strings: [("help".into(), HELP.into())].into_iter().collect(),
        flags: ["steamvr-restart".into()].into_iter().collect(),
        options: [
            HigherOrderChoiceOption {
                display_name: "Disabled".into(),
                modifiers: vec![
                    bool_modifier("session_settings.headset.controllers.enabled", true),
                    bool_modifier(
                        &format!("{PREFIX}.hand_skeleton.content.steamvr_input_2_0"),
                        false,
                    ),
                    bool_modifier(
                        &format!("{PREFIX}.hand_tracking_interaction.enabled"),
                        false,
                    ),
                ],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "SteamVR Input 2.0".into(),
                modifiers: vec![
                    bool_modifier("session_settings.headset.controllers.enabled", true),
                    bool_modifier(&format!("{PREFIX}.hand_skeleton.enabled"), true),
                    bool_modifier(
                        &format!("{PREFIX}.hand_skeleton.content.steamvr_input_2_0"),
                        true,
                    ),
                    bool_modifier(
                        &format!("{PREFIX}.hand_tracking_interaction.enabled"),
                        false,
                    ),
                ],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "ALVR bindings".into(),
                modifiers: vec![
                    bool_modifier("session_settings.headset.controllers.enabled", true),
                    bool_modifier(
                        &format!("{PREFIX}.hand_skeleton.content.steamvr_input_2_0"),
                        false,
                    ),
                    bool_modifier(&format!("{PREFIX}.hand_tracking_interaction.enabled"), true),
                ],
                content: None,
            },
        ]
        .into_iter()
        .collect(),
        default_option_display_name: "SteamVR Input 2.0".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

pub fn eye_face_tracking_schema() -> PresetSchemaNode {
    PresetSchemaNode::HigherOrderChoice(HigherOrderChoiceSchema {
        name: "Eye and face tracking".into(),
        strings: HashMap::new(),
        flags: HashSet::new(),
        options: [
            HigherOrderChoiceOption {
                display_name: "Disabled".into(),
                modifiers: vec![bool_modifier(
                    "session_settings.headset.face_tracking.enabled",
                    false,
                )],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "VRChat Eye OSC".into(),
                modifiers: vec![
                    bool_modifier("session_settings.headset.face_tracking.enabled", true),
                    bool_modifier(
                        "session_settings.headset.face_tracking.content.sink.social_presence.enabled",
                        true,
                    ),
                    string_modifier(
                        "session_settings.headset.face_tracking.content.sink.social_presence.content.variant",
                        "VrchatEyeOsc",
                    ),
                ],
                content: None,
            },
            HigherOrderChoiceOption {
                display_name: "VRCFaceTracking".into(),
                modifiers: vec![
                    bool_modifier("session_settings.headset.face_tracking.enabled", true),
                    bool_modifier(
                        "session_settings.headset.face_tracking.content.sink.social_presence.enabled",
                        true,
                    ),
                    string_modifier(
                        "session_settings.headset.face_tracking.content.sink.social_presence.content.variant",
                        "VrcFaceTracking",
                    ),
                ],
                content: None,
            },
        ]
        .into_iter()
        .collect(),
        default_option_display_name: "Disabled".into(),
        gui: ChoiceControlType::ButtonGroup,
    })
}

#[cfg(test)]
mod vrx_tests {
    use super::*;

    fn apply(preset: &HigherOrderChoiceOption, session: &mut serde_json::Value) {
        for modifier in &preset.modifiers {
            let pointer = format!("/{}", modifier.target_path.replace('.', "/"));
            let target = session
                .pointer_mut(&pointer)
                .unwrap_or_else(|| panic!("Invalid resolution preset path: {pointer}"));
            let PresetModifierOperation::Assign(value) = &modifier.operation;
            *target = value.clone();
        }
    }

    #[test]
    fn vrx_native_and_80_percent_are_independent_square_resolutions() {
        let PresetSchemaNode::HigherOrderChoice(schema) = resolution_schema() else {
            panic!("Expected resolution presets")
        };
        let vrx = schema
            .options
            .iter()
            .filter(|p| p.display_name.starts_with("ThinkReality VRX"))
            .collect::<Vec<_>>();
        assert_eq!(vrx.len(), 2);
        for (label, expected) in [
            ("ThinkReality VRX (native: 2280 x 2280)", 2280),
            ("ThinkReality VRX (80%: 1824 x 1824)", 1824),
        ] {
            let preset = vrx.iter().find(|p| p.display_name == label).unwrap();
            let mut value = serde_json::to_value(alvr_session::SessionConfig::default()).unwrap();
            apply(preset, &mut value);
            let session: alvr_session::SessionConfig = serde_json::from_value(value).unwrap();
            let settings = session.to_settings();
            for resolution in [
                settings.video.transcoding_view_resolution,
                settings.video.emulated_headset_view_resolution,
            ] {
                assert!(matches!(resolution, alvr_session::FrameSize::Absolute {
                    width, height: Some(height)
                } if width == expected && height == expected));
            }
        }
    }

    #[test]
    fn every_resolution_preset_preserves_all_other_session_settings() {
        let PresetSchemaNode::HigherOrderChoice(schema) = resolution_schema() else {
            panic!("Expected resolution presets")
        };
        for enable_features in [false, true] {
            let mut baseline =
                serde_json::to_value(alvr_session::SessionConfig::default()).unwrap();
            let video = &mut baseline["session_settings"]["video"];
            video["preferred_fps"] = 90.0.into();
            video["preferred_codec"]["variant"] = "H264".into();
            for feature in [
                "foveated_encoding",
                "clientside_foveation",
                "clientside_post_processing",
                "color_correction",
                "upscaling",
            ] {
                video[feature]["enabled"] = enable_features.into();
            }
            for preset in &schema.options {
                let mut actual = baseline.clone();
                apply(preset, &mut actual);
                // Restore only the two allowed fields, then compare the entire session.
                // This catches changes to identity, refresh rate, codec, backend or any
                // current/future quality, audio, tracking and connection setting.
                for field in [
                    "transcoding_view_resolution",
                    "emulated_headset_view_resolution",
                ] {
                    actual["session_settings"]["video"][field] =
                        baseline["session_settings"]["video"][field].clone();
                }
                assert_eq!(
                    actual, baseline,
                    "{} changed unrelated settings",
                    preset.display_name
                );
            }
        }
    }
}
