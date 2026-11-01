use alvr_common::{LogEntry, LogSeverity};
use alvr_gui_common::theme::{self, log_colors};
use alvr_session::Settings;
use eframe::{
    egui::{self, Frame, Label, Layout, Panel, RichText, TextWrapMode, Ui},
    emath::Align,
    epaint::Color32,
};
use rand::seq::IndexedRandom;
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use instant::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

const TIMEOUT: Duration = Duration::from_secs(5);
const NO_NOTIFICATIONS_MESSAGE: &str = "没有新通知";
const NOTIFICATION_TIPS: &[&str] = &[
    // The following tips are ordered roughtly in the order settings appear
    "如果更改设置后开始崩溃，请在“安装”页重新运行“设置向导”，然后点击“重置设置”以重置 ALVR。",
    "部分设置默认隐藏。点击设置旁的“展开”按钮可展开子菜单。",
    "建议保留 ALVR 的默认音频设置，并在任务栏通知区域更改默认音频设备。",
    "提高“视频”→“最大缓冲帧数”可能减少卡顿，但会增加延迟。",
    "某些 GPU 可能需要在 H.264 和 HEVC 之间切换，以解决崩溃或回退到软件编码的问题。",
    "如果使用 NVIDIA GPU，建议使用高码率 H.264；使用 AMD GPU 时，HEVC 画质可能更好。",
    "如果出现“白色雪花”闪烁，请将“预设”→“分辨率”设为“低”，并关闭“视频”→“注视点编码”。",
    "提高“视频”→“色彩校正”→“锐度”可能改善观感。",
    "如果外部控制器或追踪器无法与 ALVR 追踪空间同步，请在“头显”→“额外 OpenVR 属性”中添加一项，然后设置自定义“追踪系统名称字符串”。",
    "要更改控制器的外观，请设置“头显”→“控制器”→“模拟模式”。",
    "ALVR 支持自定义按键绑定！如需帮助，请访问 Discord 社区。",
    "ALVR 支持手势追踪（“预设”→“手部追踪交互”→“ALVR 绑定”）。请查看 Wiki 了解正确使用方法：https://github.com/alvr-org/ALVR/wiki/Hand-tracking-controller-bindings。",
    "如果手势追踪影响使用，可在“头显”→“控制器”→“手部追踪交互”中关闭，也可以启用“仅触摸”。",
    "可在“头显”→“控制器”→“预测”中微调控制器响应。",
    "如果显示的控制器或手部模型与实体控制器位置不一致，可调整“头显”→“控制器”→“左手柄位置/旋转偏移”；该设置影响双手。",
    "使用外部追踪器或控制器时，应将“头显”中的位置和旋转重置中心模式均设为“关闭”。",
    "可启用倾斜模式：将“头显”→“位置重置中心模式”设为“本地”，将“旋转重置中心模式”设为“倾斜”。",
    "如果画面经常出现异常，可使用“连接”→“避免视频画面异常”，以可能的卡顿换取较少画面错误。",
    "可通过“连接”→“启用连接/断开脚本”在头显连接或断开时运行自定义命令或程序。",
    "报告问题并获取日志时，请启用“其他”→“日志”→“写入磁盘”；日志位于 session_log.txt。",
    "调试时可启用“其他”→“日志”中的“记录追踪”“记录按键”和“记录触觉反馈”。可通过 WebSocket ws://localhost:8082/api/events 获取数据。",
    "报告问题并分享日志时，建议启用“其他”→“日志”→“优先显示回溯”。",
    "切换“其他”→“日志”→“显示通知提示”可快速切换提示。",
    "可以启用“其他”→“SteamVR 启动器”→“自动启动和关闭 SteamVR”。",
    "为报告问题分享录屏时，可启用“其他”→“捕获”→“循环视频文件”以限制上传文件大小。",
    // Miscellaneous
    "如果设备列表中没有头显，它可能位于其他子网；可使用头显中显示的 IP 地址“手动添加设备”。",
];

pub struct NotificationBar {
    message: String,
    current_level: LogSeverity,
    receive_instant: Instant,
    min_notification_level: LogSeverity,
    tip_message: Option<String>,
    expanded: bool,
}

impl NotificationBar {
    pub fn new() -> Self {
        Self {
            message: NO_NOTIFICATIONS_MESSAGE.into(),
            current_level: LogSeverity::Debug,
            receive_instant: Instant::now(),
            min_notification_level: LogSeverity::Debug,
            tip_message: None,
            expanded: false,
        }
    }

    pub fn update_settings(&mut self, settings: &Settings) {
        self.min_notification_level = settings.extra.logging.notification_level;

        if settings.extra.logging.show_notification_tip {
            if self.tip_message.is_none() {
                self.tip_message = NOTIFICATION_TIPS
                    .choose(&mut rand::rng())
                    .map(|s| format!("提示：{s}"));
            }
        } else {
            self.tip_message = None;
        }
    }

    pub fn push_notification(&mut self, event: LogEntry, from_dashboard: bool) {
        let now = Instant::now();
        let min_severity = if from_dashboard {
            if cfg!(debug_assertions) {
                LogSeverity::Debug
            } else {
                LogSeverity::Info
            }
        } else {
            self.min_notification_level
        };

        if event.severity >= min_severity
            && (now > self.receive_instant + TIMEOUT || event.severity >= self.current_level)
        {
            self.message = event.content;
            self.current_level = event.severity;
            self.receive_instant = now;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let now = Instant::now();
        if now > self.receive_instant + TIMEOUT {
            self.message = self
                .tip_message
                .clone()
                .unwrap_or_else(|| NO_NOTIFICATIONS_MESSAGE.into());
            self.current_level = LogSeverity::Debug;
        }

        let (fg, bg) = match self.current_level {
            LogSeverity::Error => (Color32::BLACK, log_colors::ERROR_LIGHT),
            LogSeverity::Warning => (Color32::BLACK, log_colors::WARNING_LIGHT),
            LogSeverity::Info => (Color32::BLACK, log_colors::INFO_LIGHT),
            LogSeverity::Debug => (theme::FG, theme::LIGHTER_BG),
        };

        let mut bottom_bar = Panel::bottom("bottom_panel").frame(
            Frame::default()
                .inner_margin(egui::vec2(10.0, 5.0))
                .fill(bg),
        );
        let alignment = if !self.expanded {
            bottom_bar = bottom_bar.max_size(26.0);

            Align::TOP
        } else {
            Align::Center
        };
        let wrapping = if !self.expanded {
            TextWrapMode::Truncate
        } else {
            TextWrapMode::Wrap
        };

        bottom_bar.show(ui, |ui| {
            ui.with_layout(Layout::right_to_left(alignment), |ui| {
                if !self.expanded {
                    if ui.small_button("展开").clicked() {
                        self.expanded = true;
                    }
                } else if ui.button("收起").clicked() {
                    self.expanded = false;
                }
                ui.with_layout(Layout::left_to_right(alignment), |ui| {
                    //A LayoutJob that has its TextWrapping updated to fill the available space would probably be a more elegant solution.
                    ui.add(
                        Label::new(RichText::new(&self.message).color(fg).size(12.0))
                            .wrap_mode(wrapping),
                    );
                })
            })
        });
    }
}
