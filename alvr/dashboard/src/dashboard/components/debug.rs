use crate::dashboard::ServerRequest;
use eframe::egui::Ui;

pub fn debug_tab_ui(ui: &mut Ui) -> Option<ServerRequest> {
    let mut request = None;

    ui.label(
        "使用下方按钮进行的 ALVR 录制不适合录制游戏画面。\n录制游戏请使用其他方式，例如通过头显或桌面 VR 输出录制。",
    );

    ui.columns(3, |ui| {
        if ui[0].button("请求 IDR 关键帧").clicked() {
            request = Some(ServerRequest::InsertIdr);
        }

        if ui[1].button("开始录制").clicked() {
            request = Some(ServerRequest::StartRecording);
        }

        if ui[2].button("停止录制").clicked() {
            request = Some(ServerRequest::StopRecording);
        }
    });

    request
}
