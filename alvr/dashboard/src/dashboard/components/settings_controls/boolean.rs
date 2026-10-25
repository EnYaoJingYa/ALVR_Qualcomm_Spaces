use super::{NestingInfo, reset};
use alvr_localization::tr;
use alvr_packets::PathValuePair;
use eframe::{
    egui::{Layout, Ui},
    emath::Align,
};
use serde_json as json;

pub struct Control {
    nesting_info: NestingInfo,
    default: bool,
}

impl Control {
    pub fn new(nesting_info: NestingInfo, default: bool) -> Self {
        Self {
            nesting_info,
            default,
        }
    }

    pub fn ui(
        &self,
        ui: &mut Ui,
        session_fragment: &mut json::Value,
        allow_inline: bool,
    ) -> Option<PathValuePair> {
        super::grid_flow_inline(ui, allow_inline);

        let json::Value::Bool(enabled_mut) = session_fragment else {
            unreachable!()
        };

        let mut request = None;

        fn get_request(nesting_info: &NestingInfo, enabled: bool) -> Option<PathValuePair> {
            Some(PathValuePair {
                path: nesting_info.path.clone(),
                value: json::Value::Bool(enabled),
            })
        }

        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if alvr_gui_common::switch(ui, enabled_mut).clicked() {
                request = get_request(&self.nesting_info, *enabled_mut);
            }

            let default_string = if self.default { tr("ON") } else { tr("OFF") };
            if reset::reset_button(ui, *enabled_mut != self.default, &default_string).clicked() {
                request = get_request(&self.nesting_info, self.default);
            }
        });

        request
    }
}
