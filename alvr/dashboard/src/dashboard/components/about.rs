use alvr_common::ALVR_VERSION;
use alvr_gui_common::theme;
use alvr_localization::tr;
use eframe::egui::{Frame, RichText, ScrollArea, Ui};

pub fn about_tab_ui(ui: &mut Ui) {
    ui.label(RichText::new(format!("{} v{}", tr("ALVR streamer"), *ALVR_VERSION)).size(30.0));
    ui.add_space(10.0);
    ui.hyperlink_to(tr("Visit us on GitHub"), "https://github.com/alvr-org/ALVR");
    ui.hyperlink_to(tr("Join us on Discord"), "https://discord.gg/ALVR");
    ui.hyperlink_to(
        tr("Latest release"),
        "https://github.com/alvr-org/ALVR/releases/latest",
    );
    ui.hyperlink_to(
        tr("Donate to ALVR on Open Collective"),
        "https://opencollective.com/alvr",
    );
    ui.add_space(10.0);
    ui.label(tr("License:"));
    Frame::group(ui.style())
        .fill(theme::DARKER_BG)
        .inner_margin(theme::FRAME_PADDING)
        .show(ui, |ui| {
            ScrollArea::new([false, true])
                .id_salt("license_scroll")
                .show(ui, |ui| ui.label(include_str!("../../../../../LICENSE")))
        });
}
