use crate::{InstallationInfo, Progress, ReleaseChannelsInfo, UiMessage, WorkerMessage, actions};
use alvr_gui_common::ModalButton;
use eframe::{
    egui::{
        self, Button, CentralPanel, ComboBox, Context, Frame, Grid, Layout, ProgressBar, RichText,
        Ui, ViewportCommand,
    },
    emath::Align,
    epaint::Color32,
};
use std::{
    mem,
    sync::mpsc::{Receiver, Sender},
};

enum State {
    Default,
    Installing(Progress),
    Error(String),
}

#[derive(Default)]
enum PopupType {
    #[default]
    None,
    DeleteInstallation(String),
    EditVersion(String),
    AddVersion {
        version_selection: Version,
        session_version_selection: Option<String>,
    },
}

#[derive(Clone, PartialEq, Eq)]
enum ReleaseChannelType {
    Stable,
    Nightly,
}

#[derive(Clone, PartialEq, Eq)]
struct Version {
    string: String,
    release_channel: ReleaseChannelType,
}

pub struct Launcher {
    worker_message_receiver: Receiver<WorkerMessage>,
    ui_message_sender: Sender<UiMessage>,
    state: State,
    release_channels_info: Option<ReleaseChannelsInfo>,
    installations: Vec<InstallationInfo>,
    popup: PopupType,
}

impl Launcher {
    pub fn new(
        cc: &eframe::CreationContext,
        worker_message_receiver: Receiver<WorkerMessage>,
        ui_message_sender: Sender<UiMessage>,
    ) -> Self {
        alvr_gui_common::theme::set_theme(&cc.egui_ctx);

        Self {
            worker_message_receiver,
            ui_message_sender,
            state: State::Default,
            release_channels_info: None,
            installations: actions::get_installations(),
            popup: PopupType::None,
        }
    }

    fn version_popup(
        &self,
        ctx: &Context,
        mut version: Version,
        mut session_version: Option<String>,
    ) -> PopupType {
        let response = alvr_gui_common::modal(
            ctx,
            "添加版本",
            {
                // Safety: unwrap is safe because the "Add release" button is available after populating the release_channels_info.
                let release_channels_info = self.release_channels_info.as_ref().unwrap();
                Some(|ui: &mut Ui| {
                    let version_str = version.string.clone();
                    let versions: Vec<_> = match &version.release_channel {
                        ReleaseChannelType::Stable => release_channels_info
                            .stable
                            .iter()
                            .map(|release| Version {
                                string: release.version.clone(),
                                release_channel: ReleaseChannelType::Stable,
                            })
                            .collect(),

                        ReleaseChannelType::Nightly => release_channels_info
                            .nightly
                            .iter()
                            .map(|release| Version {
                                string: release.version.clone(),
                                release_channel: ReleaseChannelType::Nightly,
                            })
                            .collect(),
                    };
                    let installations_with_session: Vec<_> = self
                        .installations
                        .iter()
                        .filter(|installation| installation.has_session_json)
                        .map(|installation| installation.version.clone())
                        .collect();

                    Grid::new("add-version-grid").num_columns(2).show(ui, |ui| {
                        ui.label("发布渠道");
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let channel_str = match version.release_channel {
                                ReleaseChannelType::Stable => "稳定版",
                                ReleaseChannelType::Nightly => "每日构建版",
                            };

                            ComboBox::from_id_salt("channel")
                                .selected_text(channel_str)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut version,
                                        Version {
                                            string: release_channels_info.stable[0].version.clone(),
                                            release_channel: ReleaseChannelType::Stable,
                                        },
                                        "稳定版",
                                    );
                                    ui.selectable_value(
                                        &mut version,
                                        Version {
                                            string: release_channels_info.nightly[0]
                                                .version
                                                .clone(),
                                            release_channel: ReleaseChannelType::Nightly,
                                        },
                                        "每日构建版",
                                    );
                                })
                        });
                        ui.end_row();

                        ui.label("版本");
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ComboBox::from_id_salt("version")
                                .selected_text(version_str)
                                .show_ui(ui, |ui| {
                                    for ver in versions {
                                        ui.selectable_value(&mut version, ver.clone(), ver.string);
                                    }
                                })
                        });
                        ui.end_row();

                        if cfg!(windows) {
                            ui.label("从以下版本复制配置：");
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                ComboBox::from_id_salt("session")
                                    .selected_text(session_version.clone().unwrap_or("无".into()))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut session_version, None, "无");
                                        for ver_str in installations_with_session {
                                            ui.selectable_value(
                                                &mut session_version,
                                                Some(ver_str.clone()),
                                                ver_str,
                                            );
                                        }
                                    })
                            });
                            ui.end_row();
                        }
                    });
                })
            },
            &[ModalButton::Cancel, ModalButton::Custom("安装".into())],
            None,
        );

        match response {
            Some(ModalButton::Cancel) => PopupType::None,
            Some(ModalButton::Custom(_)) => {
                let release_info = match &version.release_channel {
                    ReleaseChannelType::Stable => self
                        .release_channels_info
                        .as_ref()
                        .unwrap()
                        .stable
                        .iter()
                        .find(|release| release.version == version.string)
                        .unwrap()
                        .clone(),
                    ReleaseChannelType::Nightly => self
                        .release_channels_info
                        .as_ref()
                        .unwrap()
                        .nightly
                        .iter()
                        .find(|release| release.version == version.string)
                        .unwrap()
                        .clone(),
                };

                self.ui_message_sender
                    .send(UiMessage::InstallServer {
                        release_info,
                        session_version,
                    })
                    .ok();

                PopupType::None
            }
            _ => PopupType::AddVersion {
                version_selection: version,
                session_version_selection: session_version,
            },
        }
    }

    fn edit_popup(&self, ctx: &Context, version: String) -> PopupType {
        let mut delete_version = false;
        let response = alvr_gui_common::modal(
            ctx,
            "编辑版本",
            Some(|ui: &mut Ui| {
                ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                    delete_version = ui.button("删除版本").clicked();
                });
            }),
            &[ModalButton::Close],
            None,
        );

        if delete_version {
            PopupType::DeleteInstallation(version)
        } else if matches!(response, Some(ModalButton::Close)) {
            PopupType::None
        } else {
            PopupType::EditVersion(version)
        }
    }

    fn delete_popup(&mut self, ctx: &Context, version: String) -> PopupType {
        let response = alvr_gui_common::modal(
            ctx,
            "确定要继续吗？",
            Some({
                let version = version.clone();
                move |ui: &mut Ui| {
                    ui.with_layout(Layout::top_down(Align::Center), |ui| {
                        ui.label(format!("这将永久删除版本 {version}"));
                    });
                }
            }),
            &[ModalButton::Cancel, ModalButton::Custom("删除版本".into())],
            None,
        );

        match response {
            Some(ModalButton::Cancel) => PopupType::None,
            Some(ModalButton::Custom(_)) => {
                if let Err(e) = actions::delete_installation(&version) {
                    self.state = State::Error(format!("删除版本失败：{e}"));
                }

                self.installations = actions::get_installations();

                PopupType::None
            }
            _ => PopupType::DeleteInstallation(version),
        }
    }
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        while let Ok(msg) = self.worker_message_receiver.try_recv() {
            match msg {
                WorkerMessage::ReleaseChannelsInfo(data) => self.release_channels_info = Some(data),
                WorkerMessage::ProgressUpdate(progress) => {
                    self.state = State::Installing(progress);
                }
                WorkerMessage::Done => {
                    // Refresh installations
                    self.installations = actions::get_installations();
                    self.state = State::Default;
                }
                WorkerMessage::Error(e) => self.state = State::Error(e),
            }
        }

        CentralPanel::default().show(ui, |ui| match &self.state {
            State::Default => {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.label(RichText::new("ALVR 启动器").size(25.0).strong());
                    ui.label(match &self.release_channels_info {
                        Some(data) => format!("最新稳定版：{}", data.stable[0].version),
                        None => "正在获取最新版本…".into(),
                    });

                    for installation in &self.installations {
                        let path = actions::installations_dir().join(&installation.version);

                        Frame::group(ui.style())
                            .fill(alvr_gui_common::theme::SECTION_BG)
                            .inner_margin(egui::vec2(10.0, 5.0))
                            .show(ui, |ui| {
                                Grid::new(&installation.version)
                                    .num_columns(2)
                                    .show(ui, |ui| {
                                        ui.label(&installation.version);
                                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                            if ui.button("编辑").clicked() {
                                                self.popup = PopupType::EditVersion(
                                                    installation.version.clone(),
                                                );
                                            }

                                            if ui.button("打开目录").clicked() {
                                                open::that_in_background(path);
                                            }

                                            let release_info = self
                                                .release_channels_info
                                                .as_ref()
                                                .and_then(|info| {
                                                    actions::get_release(
                                                        info,
                                                        &installation.version,
                                                    )
                                                });
                                            if ui
                                                .add_enabled(
                                                    release_info.is_some()
                                                        || installation.is_apk_downloaded,
                                                    Button::new("安装 APK"),
                                                )
                                                .clicked()
                                            {
                                                if let Some(release_info) = release_info {
                                                    self.ui_message_sender
                                                        .send(UiMessage::InstallClient(
                                                            release_info,
                                                        ))
                                                        .ok();
                                                } else {
                                                    self.state =
                                                        State::Error("获取版本信息失败".into());
                                                }
                                            };

                                            if ui.button("启动").clicked() {
                                                match actions::launch_dashboard(
                                                    &installation.version,
                                                ) {
                                                    Ok(()) => {
                                                        self.ui_message_sender
                                                            .send(UiMessage::Quit)
                                                            .ok();
                                                        ui.send_viewport_cmd(
                                                            ViewportCommand::Close,
                                                        );
                                                    }
                                                    Err(e) => {
                                                        self.state = State::Error(e.to_string());
                                                    }
                                                }
                                            }
                                        })
                                    })
                            });
                    }

                    if ui
                        .add_enabled(
                            self.release_channels_info.is_some(),
                            Button::new("添加版本"),
                        )
                        .clicked()
                    {
                        self.popup = PopupType::AddVersion {
                            version_selection: Version {
                                string: self.release_channels_info.as_ref().unwrap().stable[0]
                                    .version
                                    .clone(),
                                release_channel: ReleaseChannelType::Stable,
                            },
                            session_version_selection: None,
                        };
                    }

                    let popup = match mem::take(&mut self.popup) {
                        PopupType::AddVersion {
                            version_selection,
                            session_version_selection,
                        } => self.version_popup(ui, version_selection, session_version_selection),
                        PopupType::EditVersion(version) => self.edit_popup(ui, version),
                        PopupType::DeleteInstallation(version) => self.delete_popup(ui, version),
                        PopupType::None => PopupType::None,
                    };
                    self.popup = popup;
                });
            }
            State::Installing(progress) => {
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.label(&progress.message);
                    ui.add(ProgressBar::new(progress.progress).animate(true));
                });
            }
            State::Error(e) => {
                let e = e.clone(); // Avoid borrowing issues with the closure for the layout
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.colored_label(Color32::LIGHT_RED, "错误！");
                    ui.label(e);

                    if ui.button("关闭").clicked() {
                        self.state = State::Default;
                    }
                });
            }
        });

        if ui.input(|i| i.viewport().close_requested()) {
            self.ui_message_sender.send(UiMessage::Quit).ok();
        }
    }
}
