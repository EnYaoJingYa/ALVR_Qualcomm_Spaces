use super::{
    NestingInfo, SettingControl,
    presets::{PresetControl, builtin_schema},
};
use crate::dashboard::ServerRequest;
use alvr_gui_common::{DisplayString, theme};
use alvr_localization::tr;
use alvr_session::{SessionSettings, Settings};
use eframe::egui::{Align, Frame, Grid, Layout, RichText, ScrollArea, Ui};
#[cfg(target_arch = "wasm32")]
use instant::Instant;
use serde_json as json;
use settings_schema::SchemaNode;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

const DATA_UPDATE_INTERVAL: Duration = Duration::from_secs(1);
const MIN_COLUMN_SIZE: f32 = 300.0;

struct TopLevelEntry {
    id: DisplayString,
    control: SettingControl,
}

pub struct SettingsTab {
    selected_top_tab_id: String,
    resolution_preset: PresetControl,
    framerate_preset: PresetControl,
    encoder_preset: PresetControl,
    foveation_preset: PresetControl,
    codec_preset: PresetControl,
    game_audio_preset: PresetControl,
    microphone_preset: PresetControl,
    hand_tracking_interaction_preset: PresetControl,
    eye_face_tracking_preset: PresetControl,
    top_level_entries: Vec<TopLevelEntry>,
    session_settings_json: Option<json::Value>,
    last_update_instant: Instant,
}

impl SettingsTab {
    pub fn new() -> Self {
        let nesting_info = NestingInfo {
            path: vec!["session_settings".into()],
            indentation_level: 0,
        };
        let schema = Settings::schema(alvr_session::session_settings_default());

        // Top level node must be a section
        let SchemaNode::Section { entries, .. } = schema else {
            unreachable!();
        };

        let top_level_entries = entries
            .into_iter()
            .map(|entry| {
                let id = entry.name;
                let display = super::get_display_name(&id, &entry.strings);

                let mut nesting_info = nesting_info.clone();
                nesting_info.path.push(id.clone().into());

                TopLevelEntry {
                    id: DisplayString { id, display },
                    control: SettingControl::new(nesting_info, entry.content),
                }
            })
            .collect();

        Self {
            selected_top_tab_id: "presets".into(),
            resolution_preset: PresetControl::new(builtin_schema::resolution_schema()),
            framerate_preset: PresetControl::new(builtin_schema::framerate_schema()),
            encoder_preset: PresetControl::new(builtin_schema::encoder_preset_schema()),
            foveation_preset: PresetControl::new(builtin_schema::foveation_preset_schema()),
            codec_preset: PresetControl::new(builtin_schema::codec_preset_schema()),
            game_audio_preset: PresetControl::new(builtin_schema::game_audio_schema()),
            microphone_preset: PresetControl::new(builtin_schema::microphone_schema()),
            hand_tracking_interaction_preset: PresetControl::new(
                builtin_schema::hand_tracking_interaction_schema(),
            ),
            eye_face_tracking_preset: PresetControl::new(builtin_schema::eye_face_tracking_schema()),
            top_level_entries,
            session_settings_json: None,
            last_update_instant: Instant::now(),
        }
    }

    pub fn update_session(&mut self, session_settings: &SessionSettings) {
        let settings_json = json::to_value(session_settings).unwrap();

        self.resolution_preset
            .update_session_settings(&settings_json);
        self.framerate_preset
            .update_session_settings(&settings_json);
        self.encoder_preset.update_session_settings(&settings_json);
        self.foveation_preset
            .update_session_settings(&settings_json);
        self.codec_preset.update_session_settings(&settings_json);
        self.game_audio_preset
            .update_session_settings(&settings_json);
        self.microphone_preset
            .update_session_settings(&settings_json);
        self.hand_tracking_interaction_preset
            .update_session_settings(&settings_json);
        self.eye_face_tracking_preset
            .update_session_settings(&settings_json);

        self.session_settings_json = Some(settings_json);
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Vec<ServerRequest> {
        let mut requests = vec![];

        let now = Instant::now();
        if now > self.last_update_instant + DATA_UPDATE_INTERVAL {
            if self.session_settings_json.is_none() {
                requests.push(ServerRequest::GetSession);
            }

            self.last_update_instant = now;
        }

        let mut path_value_pairs = vec![];
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            Frame::group(ui.style())
                .fill(theme::DARKER_BG)
                .inner_margin(theme::FRAME_PADDING)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(
                            &mut self.selected_top_tab_id,
                            "presets".into(),
                            RichText::new(tr("Presets")).raised().size(15.0),
                        );
                        for entry in &mut self.top_level_entries {
                            ui.selectable_value(
                                &mut self.selected_top_tab_id,
                                entry.id.id.clone(),
                                RichText::new(tr(&entry.id.display)).raised().size(15.0),
                            );
                        }
                    })
                })
        });

        if self.selected_top_tab_id == "presets" {
            ScrollArea::new([false, true])
                .id_salt("presets_scroll")
                .show(ui, |ui| {
                    Grid::new("presets_grid")
                        .striped(true)
                        .num_columns(2)
                        .min_col_width(MIN_COLUMN_SIZE)
                        .show(ui, |ui| {
                            path_value_pairs.extend(self.resolution_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.framerate_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.encoder_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.foveation_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.codec_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.game_audio_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.microphone_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.hand_tracking_interaction_preset.ui(ui));
                            ui.end_row();

                            path_value_pairs.extend(self.eye_face_tracking_preset.ui(ui));
                            ui.end_row();
                        })
                });
        } else {
            ScrollArea::new([false, true])
                .id_salt(format!("{}_scroll", self.selected_top_tab_id))
                .show(ui, |ui| {
                    Grid::new(format!("{}_grid", self.selected_top_tab_id))
                        .striped(true)
                        .num_columns(2)
                        .min_col_width(MIN_COLUMN_SIZE)
                        .show(ui, |ui| {
                            if let Some(session_fragment) = &mut self.session_settings_json {
                                let session_fragments_mut =
                                    session_fragment.as_object_mut().unwrap();

                                let entry = self
                                    .top_level_entries
                                    .iter_mut()
                                    .find(|entry: &&mut TopLevelEntry| {
                                        entry.id.id == self.selected_top_tab_id
                                    })
                                    .unwrap();

                                let response = entry.control.ui(
                                    ui,
                                    &mut session_fragments_mut[&entry.id.id],
                                    false,
                                );

                                if let Some(response) = response {
                                    path_value_pairs.push(response);
                                }

                                ui.end_row();
                            }
                        })
                });
        }

        if !path_value_pairs.is_empty() {
            requests.push(ServerRequest::SetSessionValues(path_value_pairs));
        }

        requests
    }
}

#[cfg(test)]
mod tests {
    use alvr_localization::has_translation;
    use alvr_session::{Settings, session_settings_default};
    use settings_schema::{SchemaEntry, SchemaNode};
    use std::collections::{BTreeSet, HashMap};

    fn section_display_name(id: &str, strings: &HashMap<String, String>) -> String {
        super::super::settings_controls::get_display_name(id, strings)
    }

    fn choice_display_name(id: &str, strings: &HashMap<String, String>) -> String {
        strings.get("display_name").cloned().unwrap_or_else(|| {
            let mut chars = id.chars();
            let mut result = vec![chars.next().unwrap()];

            for c in chars {
                let lower = c.to_ascii_lowercase();
                if lower != c {
                    result.push(' ');
                }
                result.push(lower);
            }

            result.into_iter().collect()
        })
    }

    fn check_entry_strings(
        entry: &SchemaEntry<impl Sized>,
        display_name: impl Fn(&str, &HashMap<String, String>) -> String,
        missing: &mut BTreeSet<String>,
    ) {
        let label = display_name(&entry.name, &entry.strings);
        if !has_translation(&label) {
            missing.insert(format!("label: {label}"));
        }

        for key in ["display_name", "help", "notice"] {
            if let Some(text) = entry.strings.get(key) {
                if !has_translation(text) {
                    missing.insert(format!("{}: {text}", entry.name));
                }
            }
        }
    }

    fn check_node(node: &SchemaNode, missing: &mut BTreeSet<String>) {
        match node {
            SchemaNode::Section { entries, .. } => {
                for entry in entries {
                    if !entry.flags.contains("hidden") {
                        check_entry_strings(entry, section_display_name, missing);
                    }
                    // Extra OpenVR property contents are third-party SteamVR
                    // identifiers rather than ALVR UI labels.
                    if !entry.flags.contains("hidden") && entry.name != "extra_openvr_props" {
                        check_node(&entry.content, missing);
                    }
                }
            }
            SchemaNode::Choice { variants, .. } => {
                for variant in variants {
                    check_entry_strings(variant, choice_display_name, missing);
                    if let Some(content) = &variant.content {
                        check_node(content, missing);
                    }
                }
            }
            SchemaNode::Optional { content, .. } | SchemaNode::Switch { content, .. } => {
                check_node(content, missing);
            }
            SchemaNode::Array(elements) => {
                for element in elements {
                    check_node(element, missing);
                }
            }
            SchemaNode::Vector {
                default_element, ..
            } => check_node(default_element, missing),
            SchemaNode::Dictionary { default_value, .. } => check_node(default_value, missing),
            _ => {}
        }
    }

    #[test]
    fn all_runtime_setting_messages_are_translated() {
        let mut missing = BTreeSet::new();
        check_node(&Settings::schema(session_settings_default()), &mut missing);
        assert!(
            missing.is_empty(),
            "missing translations:\n{}",
            missing.into_iter().collect::<Vec<_>>().join("\n")
        );
    }
}
