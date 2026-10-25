use crate::dashboard::ServerRequest;
use alvr_localization::tr;
use eframe::egui::Ui;

pub fn debug_tab_ui(ui: &mut Ui) -> Option<ServerRequest> {
    let mut request = None;

    ui.label(tr(
        "Recording from ALVR using the buttons below is not suitable for capturing gameplay.\nFor that, use other means of recording, for example through headset or desktop VR output.",
    ));

    ui.columns(3, |ui| {
        if ui[0].button(tr("Insert IDR")).clicked() {
            request = Some(ServerRequest::InsertIdr);
        }

        if ui[1].button(tr("Start recording")).clicked() {
            request = Some(ServerRequest::StartRecording);
        }

        if ui[2].button(tr("Stop recording")).clicked() {
            request = Some(ServerRequest::StopRecording);
        }
    });

    request
}
