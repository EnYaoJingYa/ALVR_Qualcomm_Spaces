use crate::dashboard::ServerRequest;
use eframe::{
    egui::{Button, Label, Layout, RichText, Ui},
    emath::Align,
};

pub enum SetupWizardRequest {
    ServerRequest(ServerRequest),
    Close { finished: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Welcome = 0,
    ResetSettings = 1,
    HardwareRequirements = 2,
    SoftwareRequirements = 3,
    Firewall = 4,
    Recommendations = 5,
    Finished = 6,
}

fn index_to_page(index: usize) -> Page {
    match index {
        0 => Page::Welcome,
        1 => Page::ResetSettings,
        2 => Page::HardwareRequirements,
        3 => Page::SoftwareRequirements,
        4 => Page::Firewall,
        5 => Page::Recommendations,
        6 => Page::Finished,
        _ => panic!("Invalid page index"),
    }
}

fn page_content(
    ui: &mut Ui,
    subtitle: &str,
    paragraph: &str,
    interactible_content: impl FnMut(&mut Ui),
) {
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.add_space(60.0);
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            ui.add_space(60.0);
            ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                ui.add_space(15.0);
                ui.heading(RichText::new(subtitle).size(20.0));
                ui.add(Label::new(RichText::new(paragraph).size(14.0)).wrap());
                ui.add_space(30.0);
                ui.vertical_centered(interactible_content);
            });
        })
    });
}

pub struct SetupWizard {
    page: Page,
}

impl SetupWizard {
    pub fn new() -> Self {
        Self {
            page: Page::Welcome,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Option<SetupWizardRequest> {
        let mut request = None;

        ui.horizontal(|ui| {
            ui.add_space(60.0);
            ui.vertical(|ui| {
                ui.add_space(30.0);
                ui.heading(RichText::new("欢迎使用 ALVR").size(30.0));
                ui.add_space(5.0);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(15.0);
                if ui.button("❌").clicked() {
                    request = Some(SetupWizardRequest::Close { finished: false });
                }
            })
        });
        ui.separator();
        match &self.page {
            Page::Welcome => page_content(ui, "此设置向导将帮助你设置 ALVR。", "", |_| ()),
            Page::ResetSettings => page_content(
                ui,
                "重置设置",
                "建议每次更新 ALVR 后重置设置。",
                |ui| {
                    if ui.button("重置设置").clicked() {
                        request = Some(SetupWizardRequest::ServerRequest(
                            ServerRequest::UpdateSession(Box::default()),
                        ));
                    }
                },
            ),
            Page::HardwareRequirements => page_content(
                ui,
                "硬件要求",
                "ALVR 需要较新的独立显卡；低端 Intel 集成显卡可能无法使用。请确保至少有一个音频输出设备。",
                |_| (),
            ),
            Page::SoftwareRequirements => page_content(
                ui,
                "软件要求",
                if cfg!(windows) {
                    "要在 Windows 上串流头显麦克风，需要安装 Virtual Audio Cable、VB-Cable 或 Voicemeeter。"
                } else if cfg!(target_os = "linux") {
                    "需要 PipeWire（0.3.49 或更高版本）音频系统才能串流音频和使用麦克风。"
                } else {
                    "不支持的操作系统"
                },
                #[allow(unused_variables)]
                |ui| {
                    #[cfg(windows)]
                    if ui.button("下载 Virtual Audio Cable（Lite）").clicked() {
                        ui.ctx().open_url(eframe::egui::OpenUrl::same_tab(
                            "https://software.muzychenko.net/freeware/vac470lite.zip",
                        ));
                    }
                },
            ),
            Page::Firewall => page_content(
                ui,
                "防火墙",
                "要与头显通信，需要设置防火墙规则。此操作需要管理员权限！",
                |ui| {
                    if ui.button("添加防火墙规则").clicked() {
                        request = Some(SetupWizardRequest::ServerRequest(
                            ServerRequest::AddFirewallRules,
                        ));
                    }
                },
            ),
            Page::Recommendations => page_content(
                ui,
                "建议",
                "ALVR 支持多种 PC 硬件和头显，但默认设置不一定适合全部设备。如果体验不佳，请尝试调整分辨率、码率、编码器等设置。",
                |_| (),
            ),
            Page::Finished => page_content(
                ui,
                "已完成",
                "你可以随时从左侧“安装”选项卡重新启动此设置向导。",
                |_| (),
            ),
        };

        ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
            ui.add_space(30.0);
            ui.horizontal(|ui| {
                ui.add_space(15.0);
                if self.page == Page::Finished {
                    if ui.button("完成").clicked() {
                        request = Some(SetupWizardRequest::Close { finished: true });
                    }
                } else if ui.button("下一步").clicked() {
                    self.page = index_to_page(self.page as usize + 1);
                }
                if ui
                    .add_visible(self.page != Page::Welcome, Button::new("上一步"))
                    .clicked()
                {
                    self.page = index_to_page(self.page as usize - 1);
                }
            });
            ui.separator();
        });

        request
    }
}
