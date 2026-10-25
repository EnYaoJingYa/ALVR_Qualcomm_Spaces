use std::sync::atomic::{AtomicU8, Ordering};

pub const CJK_FONT: &[u8] = include_bytes!("../resources/NotoSansSC-Variable.ttf");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    English = 0,
    Chinese = 1,
}

impl Language {
    pub fn from_locale(locale: Option<&str>) -> Self {
        if locale.is_some_and(|locale| {
            let locale = locale.to_ascii_lowercase();
            locale.starts_with("zh") || locale.contains("chinese")
        }) {
            Self::Chinese
        } else {
            Self::English
        }
    }

    pub fn detect() -> Self {
        Self::from_locale(detect_locale().as_deref())
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::English => Self::Chinese,
            Self::Chinese => Self::English,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Chinese => "zh",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "english" => Some(Self::English),
            "zh" | "zh-cn" | "zh-hans" | "chinese" => Some(Self::Chinese),
            _ => None,
        }
    }
}

static LANGUAGE: AtomicU8 = AtomicU8::new(Language::English as u8);

pub fn language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        value if value == Language::Chinese as u8 => Language::Chinese,
        _ => Language::English,
    }
}

pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}

pub fn is_chinese() -> bool {
    language() == Language::Chinese
}

pub fn tr(key: &str) -> String {
    if is_chinese() {
        translated(key).unwrap_or(key).to_owned()
    } else {
        key.to_owned()
    }
}

pub fn has_translation(key: &str) -> bool {
    translated(key).is_some()
}

#[cfg(windows)]
fn detect_locale() -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }

    let lang_id = unsafe { GetUserDefaultUILanguage() };

    if lang_id & 0x03ff == 0x0004 {
        Some("zh-CN".to_owned())
    } else {
        Some(format!("lang-{lang_id:04x}"))
    }
}

#[cfg(not(windows))]
fn detect_locale() -> Option<String> {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .ok()
}

fn translated(key: &str) -> Option<&'static str> {
    const ENTRIES: &[(&str, &str)] = &[
        ("🔌  Devices", "🔌  设备"),
        ("📈  Statistics", "📈  统计"),
        ("🔧  Settings", "🔧  设置"),
        ("💾  Installation", "💾  安装"),
        ("📝  Logs", "📝  日志"),
        ("🐞  Debug", "🐞  调试"),
        ("ℹ  About", "ℹ  关于"),
        ("SteamVR is restarting", "SteamVR 正在重启"),
        ("Restart SteamVR", "重启 SteamVR"),
        ("Launch SteamVR", "启动 SteamVR"),
        ("SteamVR:", "SteamVR："),
        ("Connected", "已连接"),
        ("Disconnected", "未连接"),
        ("Connecting", "正在连接"),
        ("Streaming", "正在串流"),
        ("Disconnecting", "正在断开"),
        (
            "ALVR requires running SteamVR! Devices will not be discovered or connected.",
            "ALVR 需要先启动 SteamVR！否则无法发现或连接设备。",
        ),
        ("Edit connection", "编辑连接"),
        ("Hostname:", "主机名："),
        ("IP Addresses:", "IP 地址："),
        ("Add new", "新增"),
        ("Cancel", "取消"),
        ("Save", "保存"),
        ("Wired Connection", "有线连接"),
        ("ADB download progress", "ADB 下载进度"),
        ("New Wireless Devices", "新的无线设备"),
        ("Trust", "信任"),
        ("Trusted Wireless Devices", "已信任的无线设备"),
        ("Add device manually", "手动添加设备"),
        ("Unknown IP", "未知 IP"),
        ("Remove", "移除"),
        ("Edit", "编辑"),
        ("Run setup wizard", "运行设置向导"),
        ("Add firewall rules", "添加防火墙规则"),
        ("Remove firewall rules", "移除防火墙规则"),
        ("Registered drivers", "已注册驱动"),
        ("Register ALVR driver", "注册 ALVR 驱动"),
        ("Welcome to ALVR", "欢迎使用 ALVR"),
        (
            "This setup wizard will help you setup ALVR.",
            "本向导将帮助你完成 ALVR 的初始配置。",
        ),
        ("Reset settings", "重置设置"),
        (
            "It is recommended to reset your settings everytime you update ALVR.",
            "建议每次更新 ALVR 后重置设置。",
        ),
        ("Hardware requirements", "硬件要求"),
        (
            "ALVR requires a dedicated and recent graphics card. Low-end Intel integrated graphics may fail to work.\nMake sure you have at least one output audio device.",
            "ALVR 需要较新的独立显卡。低端 Intel 核显可能无法正常工作。\n请至少准备一个音频输出设备。",
        ),
        ("Software requirements", "软件要求"),
        (
            "To stream the headset microphone on Windows you need to install Virtual Audio Cable, VB-Cable, Voicemeeter",
            "要在 Windows 上串流头显麦克风，需要安装 Virtual Audio Cable、VB-Cable 或 Voicemeeter。",
        ),
        (
            "You need the PipeWire (0.3.49+ version) audio system to be able to stream audio and use microphone.",
            "需要 PipeWire 0.3.49 或更高版本才能串流音频并使用麦克风。",
        ),
        (
            "Download Virtual Audio Cable (Lite)",
            "下载 Virtual Audio Cable（精简版）",
        ),
        ("Firewall", "防火墙"),
        (
            "To communicate with the headset, some firewall rules need to be set.\nThis requires administrator rights!",
            "为了让 ALVR 与头显通信，需要添加防火墙规则。\n此操作需要管理员权限！",
        ),
        ("Recommendations", "使用建议"),
        (
            "ALVR supports multiple types of PC hardware and headsets but not all might work correctly with default settings. Please try tweaking different settings like resolution, bitrate, encoder and others if your ALVR experience is not optimal.",
            "ALVR 支持多种 PC 硬件和头显，但默认设置不一定适合所有设备。如果体验不理想，请尝试调整分辨率、码率、编码器等设置。",
        ),
        ("Finished", "完成"),
        (
            "You can always restart this setup wizard from the \"Installation\" tab on the left.",
            "之后可以从左侧的“安装”页面重新运行本向导。",
        ),
        ("Next", "下一步"),
        ("Back", "上一步"),
        ("Finish", "完成"),
        ("Copy all", "全部复制"),
        ("Open logs directory", "打开日志目录"),
        ("Clear all", "全部清除"),
        (
            "Recording from ALVR using the buttons below is not suitable for capturing gameplay.\nFor that, use other means of recording, for example through headset or desktop VR output.",
            "下方按钮录制的 ALVR 视频不适合用于采集游戏画面。\n请使用头显或桌面 VR 输出等其他录制方式。",
        ),
        ("Insert IDR", "插入 IDR 帧"),
        ("Start recording", "开始录制"),
        ("Stop recording", "停止录制"),
        ("Visit us on GitHub", "访问 GitHub 项目"),
        ("Join us on Discord", "加入 Discord"),
        ("Latest release", "最新版本"),
        (
            "Donate to ALVR on Open Collective",
            "在 Open Collective 上支持 ALVR",
        ),
        ("License:", "许可证："),
        ("No new notifications", "暂无新通知"),
        ("Expand", "展开"),
        ("Reduce", "收起"),
        ("Collapse", "收起"),
        ("Presets", "预设"),
        ("Preset not applied", "未应用预设"),
        ("Unimplemented UI", "界面尚未实现"),
        ("Add entry", "新增条目"),
        ("Add element", "新增元素"),
        ("default list", "默认列表"),
        ("ON", "开"),
        ("OFF", "关"),
        ("Set", "已设置"),
        ("Default", "默认"),
        (
            "Changing this setting will make SteamVR restart!\nPlease save your in-game progress first",
            "修改此设置将导致 SteamVR 重启！\n请先保存游戏进度。",
        ),
        (
            "This setting can be changed in real-time during streaming!",
            "此设置可以在串流过程中实时修改！",
        ),
        ("Language: English", "语言：English"),
        ("语言：中文", "Language: 中文"),
        ("Audio", "音频"),
        ("Video", "视频"),
        ("Headset", "头显"),
        ("Connection", "连接"),
        ("Extra", "其他"),
        ("Controllers", "控制器"),
        ("Tracking", "追踪"),
        ("Encoder", "编码器"),
        ("Decoder", "解码器"),
        ("Network", "网络"),
        ("Statistics", "统计"),
        ("Resolution", "分辨率"),
        ("Preferred framerate", "首选帧率"),
        ("Codec preset", "编解码器预设"),
        ("Encoder preset", "编码预设"),
        ("Foveation preset", "注视点渲染预设"),
        ("Headset speaker", "头显扬声器"),
        ("Headset microphone", "头显麦克风"),
        ("Hand tracking interaction", "手势追踪交互"),
        ("Eye and face tracking", "眼动与面部追踪"),
        ("System Default", "系统默认"),
        ("Disabled", "已禁用"),
        ("Enabled", "已启用"),
        ("Automatic", "自动"),
        ("Virtual Audio Cable", "Virtual Audio Cable"),
        ("VB Cable", "VB Cable"),
        ("VoiceMeeter", "VoiceMeeter"),
        ("VoiceMeeter Aux", "VoiceMeeter Aux"),
        ("VoiceMeeter VAIO3", "VoiceMeeter VAIO3"),
        ("SteamVR Input 2.0", "SteamVR Input 2.0"),
        ("ALVR bindings", "ALVR 按键绑定"),
        ("ALVR streamer", "ALVR 串流端"),
        ("VRChat Eye OSC", "VRChat 眼部 OSC"),
        ("VRCFaceTracking", "VRCFaceTracking"),
        ("Very Low (width: 3072)", "极低（宽度 3072）"),
        ("Low (width: 3712)", "低（宽度 3712）"),
        ("Medium (width: 4288)", "中（宽度 4288）"),
        ("High (width: 5184)", "高（宽度 5184）"),
        ("Ultra (width: 5632)", "超高（宽度 5632）"),
        ("Extreme (width: 6080)", "极高（宽度 6080）"),
        ("Speed", "速度"),
        ("Balanced", "均衡"),
        ("Quality", "画质"),
        ("Light", "轻"),
        ("Medium", "中"),
        ("High", "高"),
        ("Low", "低"),
        ("Maximum buffering", "最大缓冲区"),
        ("Maximum bitrate", "最大码率"),
        ("Minimum bitrate", "最小码率"),
        ("Maximum network latency", "最大网络延迟"),
        ("Maximum decoder latency", "最大解码延迟"),
        ("Preferred FPS", "首选帧率"),
        ("Force software encoding", "强制软件编码"),
        ("Encoder thread count", "编码线程数"),
        ("Enable HDR", "启用 HDR"),
        ("10-bit encoding", "10 位编码"),
        ("Eye-tracked foveated encoding", "眼动注视点编码"),
        ("Eye tracking OSC override", "眼动 OSC 覆盖"),
        ("Body Tracking", "身体追踪"),
        ("Object Tracking", "物体追踪"),
        ("Fake Vive Trackers", "虚拟 Vive 追踪器"),
        ("VRChat Body OSC", "VRChat 身体 OSC"),
        ("Prediction", "预测"),
        ("Extra OpenVR properties", "额外 OpenVR 属性"),
        (
            "Open and close SteamVR automatically",
            "自动启动和关闭 SteamVR",
        ),
        (
            "Start video recording at client connection",
            "客户端连接时开始录制视频",
        ),
        ("SteamVR Launcher", "SteamVR 启动器"),
        ("Allow untrusted HTTP", "允许不受信任的 HTTP"),
        ("Allow remote dashboard access", "允许远程访问 Dashboard"),
        ("Local OSC port", "本地 OSC 端口"),
        ("Minimum IDR interval", "最小 IDR 间隔"),
        ("Send size", "发送缓冲区"),
        ("Receive size", "接收缓冲区"),
        (
            "No statistics available. Start SteamVR and connect to a device to gather statistics.",
            "暂无统计数据。请启动 SteamVR 并连接设备。",
        ),
        ("Latency", "延迟"),
        ("Motion to Photon Latency", "运动到成像延迟"),
        ("ALVR Latency", "ALVR 延迟"),
        (
            "Client System (not ALVR latency)",
            "客户端系统（非 ALVR 延迟）",
        ),
        ("Client App Compositor", "客户端应用合成"),
        ("Frame Buffering", "帧缓冲"),
        ("Decode", "解码"),
        ("Encode", "编码"),
        ("Streamer Compositor", "串流端合成"),
        ("Game Render (not ALVR latency)", "游戏渲染（非 ALVR 延迟）"),
        ("Framerate", "帧率"),
        ("Server FPS", "串流端 FPS"),
        ("Client FPS", "客户端 FPS"),
        ("Bitrate and Throughput", "码率与吞吐量"),
        ("Initial calculated throughput", "初始计算吞吐量"),
        ("Encoder latency limiter", "编码延迟限制"),
        ("Network latency limiter", "网络延迟限制"),
        ("Decoder latency limiter", "解码延迟限制"),
        ("Manual max throughput", "手动最大吞吐量"),
        ("Manual min throughput", "手动最小吞吐量"),
        ("Requested bitrate", "请求码率"),
        ("Recorded throughput", "实际吞吐量"),
        ("Recorded bitrate", "实际码率"),
        ("Total packets:", "总数据包："),
        ("Total sent:", "总发送量："),
        ("Bitrate:", "码率："),
        ("Total latency:", "总延迟："),
        ("Encoder latency:", "编码延迟："),
        ("Transport latency:", "传输延迟："),
        ("Decoder latency:", "解码延迟："),
        ("Client FPS:", "客户端 FPS："),
        ("Streamer FPS:", "串流端 FPS："),
        ("Headset battery", "头显电量"),
        ("plugged", "充电中"),
        ("unplugged", "未充电"),
        (
            "Searching for streamer...\nOpen ALVR on your PC then click \"Trust\"\nnext to the device entry",
            "正在搜索串流端...\n请在 PC 上打开 ALVR，然后点击设备旁的“信任”",
        ),
        (
            "Successful connection!\nPlease wait...",
            "连接成功！\n请稍候...",
        ),
        (
            "The stream will begin soon\nPlease wait...",
            "串流即将开始\n请稍候...",
        ),
        (
            "The streamer is restarting\nPlease wait...",
            "串流端正在重启\n请稍候...",
        ),
        ("The streamer has disconnected.", "串流端已断开连接。"),
        ("Connection timeout.", "连接超时。"),
        ("Unexpected packet", "收到意外数据包"),
        ("OK", "确定"),
        ("Close", "关闭"),
        ("Unsupported OS", "不支持的操作系统"),
        ("Don't remind me again for this version", "此版本不再提醒"),
        ("New ALVR version available", "发现新的 ALVR 版本"),
        (
            "You can download this version using the launcher:",
            "你可以使用启动器下载此版本：",
        ),
        ("Open Launcher", "打开启动器"),
        ("Download Launcher", "下载启动器"),
        ("Releases page", "版本发布页"),
        (
            "Note: throughput is the peak bitrate, packet_size/network_latency.",
            "说明：吞吐量是峰值码率，即 packet_size / network_latency。",
        ),
        ("Very low", "极低"),
        ("1/4 resolution", "四分之一分辨率"),
        ("Enable High-Motion Quality Boost", "启用高动态画质增强"),
        ("Enable Pre-analysis", "启用预分析"),
        ("Force HDR sRGB Correction", "强制 HDR sRGB 校正"),
        ("Clamp HDR extended range", "限制 HDR 扩展范围"),
        ("Quality preset", "质量预设"),
        ("Enable VBAQ/CAQ", "启用 VBAQ/CAQ"),
        ("h264: Profile", "H.264 配置档"),
        ("Encoding Gamma", "编码 Gamma"),
        ("HDR", "HDR"),
        ("NVENC", "NVENC"),
        ("AMF", "AMF"),
        ("Software (CPU) encoding", "软件（CPU）编码"),
        ("Type", "类型"),
        ("latency overstep", "延迟超限"),
        ("Constant", "固定"),
        ("FPS reset threshold multiplier", "FPS 重置阈值倍率"),
        ("Foveation offset", "注视点偏移"),
        ("Center region size", "中心区域大小"),
        ("Center shift", "中心偏移"),
        ("Baseline", "Baseline"),
        ("Hue start max", "色相起点最大值"),
        ("Hue start min", "色相起点最小值"),
        ("Hue end min", "色相终点最小值"),
        ("Hue end max", "色相终点最大值"),
        ("RGB Chroma Key", "RGB 色度键"),
        ("HSV Chroma Key", "HSV 色度键"),
        ("Client-side foveation", "客户端注视点渲染"),
        ("Client-side post-processing", "客户端后处理"),
        ("By name (substring)", "按名称（子串）"),
        ("By index", "按索引"),
        ("Average buffering", "平均缓冲"),
        ("Batch size", "批处理大小"),
        ("Mute desktop audio when streaming", "串流时静音桌面音频"),
        ("Power Saving", "省电"),
        ("Sustained Low", "持续低功耗"),
        ("Sustained High", "持续高性能"),
        ("CPU", "CPU"),
        ("GPU", "GPU"),
        ("Minimum duration", "最短持续时间"),
        ("SteamVR input 2.0", "SteamVR Input 2.0"),
        (
            "Map non-held controllers to SteamVR trackers",
            "将未握持的控制器映射为 SteamVR 追踪器",
        ),
        ("VMC", "VMC"),
        ("UDP", "UDP"),
        ("TCP", "TCP"),
        ("DSCP (packet prio hints)", "DSCP（数据包优先级提示）"),
        ("Duration", "持续时间"),
        ("Quest 1", "Quest 1"),
        ("Quest 2", "Quest 2"),
        ("Quest Pro", "Quest Pro"),
        ("Pico 4", "Pico 4"),
        ("Rift S", "Rift S"),
        ("Rift S Touch", "Rift S Touch"),
        ("Quest 1 Touch", "Quest 1 Touch"),
        ("Quest 2 Touch", "Quest 2 Touch"),
        ("Quest 3 Touch Plus", "Quest 3 Touch Plus"),
        ("PSVR2 Sense Controller", "PSVR2 Sense 控制器"),
        ("Valve Index", "Valve Index"),
        ("Vive Wand", "Vive 手柄"),
        ("Vive Tracker", "Vive 追踪器"),
        ("CBR", "CBR（固定码率）"),
        ("VBR", "VBR（可变码率）"),
        ("CAVLC", "CAVLC"),
        ("CABAC", "CABAC"),
        ("H264", "H.264"),
        ("60Hz", "60 Hz"),
        ("72Hz", "72 Hz"),
        ("80Hz", "80 Hz"),
        ("90Hz", "90 Hz"),
        ("120Hz", "120 Hz"),
        (
            "Choosing too high resolution (commonly 'High (width: 5184)') may result in high latency or black screen.",
            "选择过高的分辨率（通常是“高（宽度 5184）”）可能导致高延迟或黑屏。",
        ),
        (
            "Selecting a quality too high may result in stuttering or still image!",
            "选择过高的质量可能导致卡顿或画面定格！",
        ),
        (
            "Foveation affects pixelation on the edges of the screen and significantly reduces codec latency. \nIt is not recommended to fully disable it, as it may cause shutterring and high encode/decode latency!",
            "注视点渲染会影响画面边缘的像素化程度，并显著降低编解码延迟。\n不建议完全禁用它，否则可能导致画面抖动以及较高的编解码延迟！",
        ),
        (
            "You can change the default audio device from the system taskbar tray (bottom right)",
            "可以从系统任务栏托盘（右下角）更改默认音频设备。",
        ),
        (
            "Disabled: hands cannot emulate buttons. Useful for using Joy-Cons or other non-native controllers.\nSteamVR Input 2.0: create separate SteamVR devices for hand tracking.\nALVR bindings: use ALVR hand tracking button bindings. Check the wiki for help.\n",
            "禁用：手部无法模拟按键，适合使用 Joy-Con 或其他非原生控制器。\nSteamVR Input 2.0：为手部追踪创建独立的 SteamVR 设备。\nALVR 按键绑定：使用 ALVR 手部追踪按键绑定，具体用法请参阅 Wiki。\n",
        ),
        (
            "AV1 encoding is only supported on RDNA3, Ada Lovelace, Intel ARC or newer GPUs (AMD RX 7xxx+ , NVIDIA RTX 40xx+, Intel ARC)\nand on headsets that have XR2 Gen 2 onboard (Quest 3, Pico 4 Ultra).\n\nH264 encoding is currently NOT supported on Intel ARC GPUs on Windows.",
            "AV1 编码仅支持 RDNA3、Ada Lovelace、Intel ARC 或更新显卡（AMD RX 7xxx+、NVIDIA RTX 40xx+、Intel ARC），并且头显需搭载 XR2 Gen 2（Quest 3、Pico 4 Ultra）。\n\nWindows 上的 Intel ARC 显卡目前不支持 H.264 编码。",
        ),
        (
            "P1 is the fastest preset and P7 is the preset that produces better quality. P6 and P7 are too slow to be usable.",
            "P1 速度最快，P7 画质最好；但 P6 和 P7 通常过慢，不适合实际使用。",
        ),
        (
            "Reduce compression artifacts at the cost of small performance penalty",
            "减少压缩伪影，但会带来少量性能开销。",
        ),
        (
            "Forces the encoder to use CPU instead of GPU",
            "强制编码器使用 CPU 而不是 GPU。",
        ),
        (
            "Allowed percentage of frame interval to allocate for video encoding",
            "允许分配给视频编码的帧间隔百分比。",
        ),
        (
            "When the decoder latency goes above this threshold, the bitrate will be reduced",
            "解码延迟超过此阈值时会降低码率。",
        ),
        (
            "Number of consecutive frames above the threshold to trigger a bitrate reduction",
            "连续超过阈值多少帧后触发码率降低。",
        ),
        (
            "Controls how much the bitrate is reduced when the decoder latency goes above the threshold",
            "控制解码延迟超过阈值时码率的下降幅度。",
        ),
        (
            "Percentage of network bandwidth to allocate for video transmission",
            "分配给视频传输的网络带宽百分比。",
        ),
        (
            "Resolution used for encoding and decoding. Relative to a single eye view.",
            "用于编码和解码的分辨率，以单眼画面为基准。",
        ),
        (
            "This is the resolution that SteamVR will use as default for the game rendering. Relative to a single eye view.",
            "SteamVR 默认用于游戏渲染的分辨率，以单眼画面为基准。",
        ),
        (
            "Increasing this value will help reduce stutter but it will increase latency",
            "增大此值可减少卡顿，但会增加延迟。",
        ),
        (
            "Attempts to use a software decoder on the device. Slow, but may work around broken codecs.",
            "尝试在设备上使用软件解码。速度较慢，但可绕过部分硬件解码器问题。",
        ),
        (
            "When disabling this, the client needs to be restarted for the change to be applied.",
            "禁用后需要重启客户端才能生效。",
        ),
        (
            "Forward eye and face tracking to another application",
            "将眼动和面部追踪数据转发给其他应用。",
        ),
        (
            "Turn this off to temporarily pause tracking.",
            "关闭此选项可暂时暂停追踪。",
        ),
        (
            "Turn this off to temporarily pause sending data.",
            "关闭此选项可暂时停止发送数据。",
        ),
        (
            "Maximum prediction for head and controllers. Used to avoid too much jitter during loading.",
            "头显和控制器的最大预测时间，用于避免加载期间过度抖动。",
        ),
        (
            "Allow untrusted clients to connect without confirmation. This is not recommended for security reasons.",
            "允许未受信任的客户端不经确认直接连接。出于安全考虑，不建议启用。",
        ),
        (
            "Delay in seconds to wait after booting the headset before trying to launch the client.",
            "头显启动后等待多少秒再尝试启动客户端。",
        ),
        (
            "Launches SteamVR automatically when the ALVR dashboard is opened, and closes it when the dashboard is closed.",
            "打开 ALVR 控制面板时自动启动 SteamVR，关闭控制面板时自动关闭 SteamVR。",
        ),
        (
            "Write logs into the session_log.txt file.",
            "将日志写入 session_log.txt 文件。",
        ),
        (
            "Notification tips teach you how to use ALVR",
            "通知提示会介绍 ALVR 的使用方法。",
        ),
        (
            "If you started having crashes after changing some settings, reset ALVR by re-running \"Run setup wizard\" from the \"Installation\" tab and clicking \"Reset settings\".",
            "如果修改设置后开始崩溃，请在“安装”页面重新运行设置向导，并点击“重置设置”。",
        ),
        (
            "Some settings are hidden by default. Click the \"Expand\" button next to some settings to expand the submenus.",
            "部分设置默认隐藏。点击设置旁的“展开”按钮可显示子菜单。",
        ),
        (
            "It's highly advisable to keep audio settings as default in ALVR and modify the default audio device in the taskbar tray.",
            "建议保持 ALVR 的默认音频设置，并在系统托盘中选择默认音频设备。",
        ),
        (
            "Increasing \"Video\"->\"Maximum buffering\" may reduce stutters at the cost of more latency.",
            "增大“视频”→“最大缓冲区”可减少卡顿，但会增加延迟。",
        ),
        (
            "If you're using an NVIDIA GPU, it's best to use high-bitrate H264; if you're using an AMD GPU, HEVC might look better.",
            "使用 NVIDIA 显卡时建议使用高码率 H.264；使用 AMD 显卡时 HEVC 可能画质更好。",
        ),
        (
            "To change the visual appearance of controllers, set \"Headset\"->\"Controllers\"->\"Emulation mode\".",
            "要更改控制器外观，请设置“头显”→“控制器”→“模拟模式”。",
        ),
        (
            "ALVR supports custom button bindings! If you need help, please ask us on our Discord server.",
            "ALVR 支持自定义按键绑定！如需帮助，请加入 Discord。",
        ),
        (
            "If your headset does not appear in the device list, it might be in a different subnet. Try \"Add device manually\" with IP shown from inside device.",
            "如果头显没有出现在设备列表中，可能位于不同子网。请使用头显显示的 IP 地址手动添加设备。",
        ),
        (
            "Allow cross-origin browser requests to control ALVR settings remotely.",
            "允许跨域浏览器请求远程控制 ALVR 设置。",
        ),
        (
            "Serve the dashboard on all network interfaces instead of localhost only. Anyone who can reach this machine can then control ALVR, as the web server has no authentication.",
            "在所有网络接口上提供 Dashboard，而不仅限于本机访问。任何能访问此计算机的人都可以控制 ALVR，因为 Web 服务器没有身份验证。",
        ),
        (
            "Directly start the VR server, bypassing Steam. Will run start_server.bat if it exists alongside session.json, and try to automatically find SteamVR otherwise.",
            "绕过 Steam 直接启动 VR 服务器。如果 session.json 旁存在 start_server.bat 则运行该脚本，否则尝试自动查找 SteamVR。",
        ),
        (
            "Directly start the VR server, bypassing Steam. Will run start_server.sh if it exists alongside session.json, and try to automatically find SteamVR otherwise.",
            "绕过 Steam 直接启动 VR 服务器。如果 session.json 旁存在 start_server.sh 则运行该脚本，否则尝试自动查找 SteamVR。",
        ),
        ("AV1", "AV1"),
        (
            "CAVLC algorithm is recommended.\nCABAC produces better compression but it's significantly slower and may lead to runaway latency",
            "建议使用 CAVLC 算法。\nCABAC 压缩率更高，但速度明显更慢，可能导致延迟失控。",
        ),
        (
            "CBR: Constant BitRate mode. This is recommended.\nVBR: Variable BitRate mode. Not commended because it may throw off the adaptive bitrate algorithm. This is only supported on Windows and only with AMD/Nvidia GPUs",
            "CBR：固定码率模式，推荐使用。\nVBR：可变码率模式，可能干扰自适应码率算法，因此不推荐。该模式仅在 Windows 的 AMD/NVIDIA 显卡上受支持。",
        ),
        (
            "Clamps HDR extended range to 0.0~1.0, useful if you only want HDR to reduce banding.",
            "将 HDR 扩展范围限制在 0.0~1.0。如果只想通过 HDR 减少色带，可启用此项。",
        ),
        (
            "Controls overall quality preset of the encoder. Works only on Windows AMD AMF, Linux VAAPI (AMD/Intel).",
            "控制编码器的整体质量预设。仅适用于 Windows AMD AMF 和 Linux VAAPI（AMD/Intel）。",
        ),
        (
            "Controls the smoothness during calculations",
            "控制计算过程中的平滑程度。",
        ),
        (
            "Currently there is a bug where the decoder latency keeps rising when above a certain bitrate",
            "当前存在一个缺陷：码率超过特定值后，解码延迟会持续上升。",
        ),
        (
            "Dimensional resolution multiplier, high values will cause performance issues with weaker headset hardware or higher resolutions",
            "分辨率尺寸倍率。较高数值会使性能较弱的头显或高分辨率场景出现性能问题。",
        ),
        (
            "Enables high motion quality boost mode.\nAllows the encoder to perform pre-analysis the motion of the video and use the information for better encoding",
            "启用高动态画质增强模式。\n允许编码器预分析视频运动，并利用分析结果改善编码质量。",
        ),
        (
            "Enables pre-analysis during encoding. This will likely result in reduced performance, but may increase quality.\nDoes not work with the \"Reduce color banding\" option, requires enabling \"Use preproc\"",
            "启用编码期间的预分析。这可能降低性能，但可提高画质。\n该功能与“减少色带”选项不兼容，并且需要启用“Use preproc”。",
        ),
        (
            "Enables Variance Based Adaptive Quantization on h264 and HEVC, and Content Adaptive Quantization on AV1",
            "在 H.264 和 HEVC 上启用基于方差的自适应量化，在 AV1 上启用内容自适应量化。",
        ),
        (
            "Enabling this allows using hand gestures to emulate controller inputs.",
            "启用后可使用手势模拟控制器输入。",
        ),
        (
            "Enabling this passes skeletal hand data (finger tracking) to SteamVR.",
            "启用后会把手部骨骼数据（手指追踪）传递给 SteamVR。",
        ),
        (
            "Enabling this will adapt transparency based on the brightness of each pixel.\nThis is a similar effect to AR glasses.",
            "启用后将根据每个像素的亮度调整透明度。\n效果类似于 AR 眼镜。",
        ),
        (
            "Disabling foveated encoding may result in significantly higher encode/decode times and stuttering, or even crashing.\nIf you want to reduce the amount of pixelation on the edges, increase the center region width and height.\nFor eye-tracked foveation, enable Headset > Face tracking > Sink > Eye-tracked foveated encoding.",
            "禁用注视点编码可能显著增加编解码耗时并导致卡顿，甚至崩溃。\n如果希望减少边缘像素化，请增大中心区域的宽度和高度。\n如需眼动注视点渲染，请启用“头显 > 面部追踪 > 输出 > 眼动注视点编码”。",
        ),
        (
            "Enabling this will use separate tracker objects with the full skeletal tracking level when hand tracking is detected. This is required for VRChat hand tracking.",
            "启用后，在检测到手部追踪时会创建具有完整骨骼追踪级别的独立追踪器对象。VRChat 手部追踪需要此功能。",
        ),
        (
            "Higher values make the controllers track smoother.\nTechnically, this is the time (counted in frames) between pose submitted to SteamVR and the corresponding virtual vsync happens.\nCurrently this cannot be reliably estimated automatically. The correct value should be 2 but 3 is default for smoother tracking at the cost of slight lag.",
            "较高数值会让控制器追踪更平滑。\n从技术上说，这是姿态提交给 SteamVR 到对应虚拟 VSync 之间的时间（以帧计）。\n目前无法可靠地自动估计该值。理论正确值为 2，但默认值 3 会让追踪更平滑，代价是轻微延迟。",
        ),
        (
            "Predict hand skeleton to make it less floaty. It may make hands too jittery.",
            "预测手部骨骼以减轻漂移感，但可能使手部抖动更明显。",
        ),
        (
            "Stage: the playspace origin is determined by the room-scale guardian setup.\nLocal floor: the origin is on the floor and resets when long pressing the recentering button.\nLocal: the origin resets when long pressing the recentering button, and is calculated as an offset from the current head position.\nTilted: the world gets tilted when long pressing the recentering button. This is useful for using VR while laying down.",
            "舞台：游玩空间原点由房间规模边界设置决定。\n本地地面：原点位于地面，长按重定位按钮时重置。\n本地：长按重定位按钮时，以当前头部位置为基准重新计算原点。\n倾斜：长按重定位按钮时倾斜世界，适合躺卧使用 VR。",
        ),
        (
            "This works only on Windows. It shouldn't be disabled except in certain circumstances when you know the VR game will not meet the target framerate.",
            "此功能仅适用于 Windows。除非确定 VR 游戏无法达到目标帧率，否则不应禁用。",
        ),
        (
            "To be able to use the microphone on Windows, you need to install Virtual Audio Cable",
            "要在 Windows 上使用麦克风，需要安装 Virtual Audio Cable。",
        ),
        (
            "Track hand skeleton while holding controllers. This will reduce hand tracking frequency to 30Hz.\nBecause of runtime limitations, this option is ignored when body tracking is active.",
            "握持控制器时仍追踪手部骨骼。这会将手部追踪频率降至 30Hz。\n由于运行时限制，身体追踪启用时此选项会被忽略。",
        ),
        (
            "Ensure that the specified bitrate value is respected regardless of the framerate",
            "确保无论帧率如何变化，都遵守指定的码率值。",
        ),
        (
            "Force enable on smartphone clients",
            "在智能手机客户端上强制启用。",
        ),
        (
            "Forces sRGB correction on all composited SteamVR layers. Useful if an HDR-injected game is too dark.",
            "强制对所有合成的 SteamVR 图层进行 sRGB 校正。适用于注入 HDR 后画面过暗的游戏。",
        ),
        ("h264", "H.264"),
        (
            "Hardware optimized algorithms, available on Quest and Pico headsets",
            "硬件优化算法，适用于 Quest 和 Pico 头显。",
        ),
        ("HEVC", "HEVC / H.265"),
        (
            "HEVC may provide better visual fidelity at the cost of increased encoder latency",
            "HEVC 可提供更好的视觉保真度，但会增加编码延迟。",
        ),
        (
            "Horizontal and vertical compression ratios, in X/Y order",
            "水平和垂直压缩比例，按 X/Y 顺序。",
        ),
        (
            "Horizontal and vertical shift, in X/Y order. Used for fixed foveated encoding and when eye tracking is unavailable. Eye-tracking centers replace this shift rather than adding to it.",
            "水平和垂直偏移，按 X/Y 顺序。用于固定注视点编码以及眼动追踪不可用时。眼动追踪中心会替换此偏移，而不是与其相加。",
        ),
        (
            "How close the tips of your fingers need to be to register a pinch click.",
            "手指指尖需要靠得多近才会触发捏合点击。",
        ),
        (
            "How close to your palm the tips of your fingers need to be to register a curl click.",
            "手指指尖需要离掌心多近才会触发握拳点击。",
        ),
        (
            "How close to your palm the tips of your fingers need to be to start registering a trigger pull.",
            "手指指尖需要离掌心多近才开始识别扳机拉动。",
        ),
        (
            "How close together the tips of your fingers need to be to start registering a pinch trigger pull.",
            "手指指尖需要靠得多近才开始识别捏合扳机拉动。",
        ),
        (
            "How long the after the gesture has been deactivated before it can be activated again.",
            "手势停用后需要等待多久才能再次激活。",
        ),
        (
            "How long the gesture must be continuously held before it is activated.",
            "手势需要连续保持多久才会激活。",
        ),
        (
            "How long the gesture must be continuously released before it is deactivated.",
            "手势需要连续松开多久才会停用。",
        ),
        (
            "If on_connect.bat exists alongside session.json, it will be run on headset connect. Env var ACTION will be set to `connect`.",
            "如果 session.json 旁存在 on_connect.bat，头显连接时会运行该脚本，并将环境变量 ACTION 设置为 `connect`。",
        ),
        (
            "If on_connect.sh exists alongside session.json, it will be run on headset connect. Env var ACTION will be set to `connect`.",
            "如果 session.json 旁存在 on_connect.sh，头显连接时会运行该脚本，并将环境变量 ACTION 设置为 `connect`。",
        ),
        (
            "If on_disconnect.bat exists alongside session.json, it will be run on headset disconnect. Env var ACTION will be set to `disconnect`.",
            "如果 session.json 旁存在 on_disconnect.bat，头显断开时会运行该脚本，并将环境变量 ACTION 设置为 `disconnect`。",
        ),
        (
            "If on_disconnect.sh exists alongside session.json, it will be run on headset disconnect. Env var ACTION will be set to `disconnect`.",
            "如果 session.json 旁存在 on_disconnect.sh，头显断开时会运行该脚本，并将环境变量 ACTION 设置为 `disconnect`。",
        ),
        (
            "If the client has no preference, enables compositing VR layers to an RGBA float16 framebuffer, and doing sRGB/YUV conversions in shader code.",
            "客户端未指定偏好时，将 VR 图层合成到 RGBA float16 帧缓冲，并在着色器中执行 sRGB/YUV 转换。",
        ),
        (
            "If the client, server or the network discarded one packet, discard packets until a IDR packet is found.",
            "如果客户端、串流端或网络丢弃了任一数据包，则持续丢弃数据包，直到找到 IDR 包。",
        ),
        (
            "If the framerate changes more than this factor, trigger a parameters update",
            "如果帧率变化超过此倍率，则触发参数更新。",
        ),
        (
            "If trackers have not been calibrated before, the calibration process will start after you connect to the streamer.",
            "如果追踪器尚未校准，连接串流端后将开始校准。",
        ),
        (
            "Improve clarity of high contrast edges and counteract blur.\nUseful when the input resolution is low compared to the headset display",
            "提高高对比边缘的清晰度并抵消模糊。\n当输入分辨率低于头显显示分辨率时很有用。",
        ),
        (
            "Improves accuracy of the tracking at the cost of higher latency.",
            "提高追踪精度，但会增加延迟。",
        ),
        (
            "Improves visual quality by using the edge direction to upscale at a slight performance loss",
            "利用边缘方向进行放大，以轻微性能损失换取更好的画质。",
        ),
        (
            "In CBR mode, this makes sure the bitrate does not fall below the assigned value. This is mostly useful for debugging.",
            "在 CBR 模式下确保码率不低于指定值，主要用于调试。",
        ),
        (
            "Linear and angular velocity multiplier for debug purposes.\nIt does not update in real time.",
            "用于调试的线速度和角速度倍率。\n该设置不会实时更新。",
        ),
        ("List of OpenXR-syle paths", "OpenXR 风格路径列表。"),
        ("Main", "主要（Main）"),
        (
            "Non-held controllers are mapped to left and right feet.\nThis will be configurable in the future.",
            "未握持的控制器会映射到左右脚。\n以后可对此进行配置。",
        ),
        (
            "Power Savings might increase latency or reduce framerate consistency but decreases temperatures and improves battery life.\nSustained Low provides consistent framerates but might increase latency if necessary.\nSustained High provides consistent framerates but increases temperature.\nThis is mainly for Quest headsets, mileage may vary on other devices.",
            "省电模式可能增加延迟或降低帧率稳定性，但会降低温度并延长续航。\n持续低功耗模式提供稳定帧率，但必要时可能增加延迟。\n持续高性能模式提供稳定帧率，但会提高温度。\n此设置主要面向 Quest 头显，其他设备上的效果可能不同。",
        ),
        (
            "Prefer active upper body tracking, Quest 3 only",
            "优先使用主动上半身追踪，仅支持 Quest 3。",
        ),
        (
            "Reduce flicker for high contrast edges.\nUseful when the input resolution is high compared to the headset display",
            "减少高对比边缘的闪烁。\n当输入分辨率高于头显显示分辨率时很有用。",
        ),
        (
            "Right controller offset is mirrored horizontally",
            "右控制器偏移会沿水平方向镜像。",
        ),
        (
            "Sets the encoder to use 10 bits per channel instead of 8, if the client has no preference. Does not work on Linux with Nvidia",
            "客户端未指定偏好时，将编码器从每通道 8 位改为 10 位。Linux NVIDIA 环境不支持。",
        ),
        (
            "Snapdragon Game Super Resolution client-side upscaling",
            "客户端使用 Snapdragon Game Super Resolution 进行放大。",
        ),
        (
            "Spatial: Helps reduce color banding, but high-complexity scenes might look worse.\nTemporal: Helps improve overall encoding quality, very small trade-off in speed.",
            "空间模式：有助于减少色带，但复杂场景的画质可能变差。\n时间模式：有助于提高整体编码质量，速度损失很小。",
        ),
        (
            "The radius of motion of the joystick. The joystick can be controlled if the thumb is within 2x this range.",
            "摇杆的运动半径。拇指在此范围的两倍以内时可控制摇杆。",
        ),
        (
            "The server discards video packets if it can't push them to the network.\nThis could happen on TCP. A IDR frame is requested in this case.",
            "串流端无法将视频包推送到网络时会丢弃这些包。\nTCP 下可能发生此情况，此时会请求一个 IDR 帧。",
        ),
        (
            "The threshold is applied per-channel",
            "阈值按每个通道分别应用。",
        ),
        (
            "These settings enable extra spammy logs for debugging purposes.",
            "这些设置会启用额外的详细调试日志。",
        ),
        (
            "This applies only to certain error or warning messages.",
            "仅适用于特定错误或警告消息。",
        ),
        (
            "This device is set in SteamVR as the default microphone",
            "在 SteamVR 中将此设备设为默认麦克风。",
        ),
        (
            "This device is used by ALVR to output microphone audio",
            "ALVR 使用此设备输出麦克风音频。",
        ),
        (
            "To prioritize darker pixels at the expense of potentially additional banding in midtones, set to 2.2. To allow the encoder to decide priority on its own, set to 1.0.",
            "设置为 2.2 会优先保留暗部像素，但中间调可能出现更多色带；设置为 1.0 则由编码器自行决定。",
        ),
        (
            "Turning this off will make the controllers appear powered off.",
            "关闭后控制器会显示为已断电。",
        ),
        (
            "UDP: Faster, but less stable than TCP. Try this if your network is well optimized and free of interference.\nTCP: Slower than UDP, but more stable. Pick this if you experience video or audio stutters with UDP.",
            "UDP：速度比 TCP 快，但稳定性较低。如果网络质量良好且干扰较少，可尝试此项。\nTCP：速度比 UDP 慢，但更稳定。如果使用 UDP 时出现视频或音频卡顿，可选择此项。",
        ),
        (
            "Use external combined gaze received over OSC/UDP on localhost instead of headset eye directions. This input is shared by foveated encoding and social output. Turn this off to use headset eye tracking.",
            "使用通过本机 OSC/UDP 收到的外部合成注视数据，代替头显眼动方向。此输入同时用于注视点编码和社交输出。关闭后将使用头显眼动追踪。",
        ),
        (
            "Use the selected eye tracking input to move the foveation center. Requires Video > Foveated encoding to be enabled. Turn this off to keep fixed foveated encoding.",
            "使用选定的眼动追踪输入移动注视点中心。需要启用“视频 > 注视点编码”。关闭后将保持固定注视点编码。",
        ),
        (
            "Wether ALVR should try to automatically launch the client when establishing a wired connection.",
            "建立有线连接时，ALVR 是否应尝试自动启动客户端。",
        ),
        (
            "When this is enabled, an IDR frame is requested after the bitrate is changed.\nThis has an effect only on AMD GPUs.",
            "启用后会在码率变更后请求一个 IDR 帧。\n此设置仅影响 AMD 显卡。",
        ),
        (
            "Whenever possible, attempts to use this profile. May increase compatibility with varying mobile devices. Only has an effect for h264. Doesn't affect NVENC on Windows.",
            "尽可能使用此配置档，可提高与不同移动设备的兼容性。仅对 H.264 生效，不影响 Windows 上的 NVENC。",
        ),
        (
            "Which release type of client should ALVR look for when establishing a wired connection.",
            "建立有线连接时，ALVR 应查找哪种发布类型的客户端。",
        ),
        (
            "Width and height, in X/Y order",
            "宽度和高度，按 X/Y 顺序。",
        ),
        (
            "You probably don't want to change this. Allows for changing adapter for ALVR compositor.",
            "通常无需修改。此设置允许为 ALVR 合成器更改适配器。",
        ),
        (
            "Sometimes switching between h264 and HEVC codecs is necessary on certain GPUs to fix crashing or fallback to software encoding.",
            "在某些显卡上，可能需要在 H.264 和 HEVC 编码器之间切换，以解决崩溃或回退到软件编码的问题。",
        ),
        (
            "If you experience \"white snow\" flickering, set \"Presets\"->\"Resolution\" to \"Low\" and disable \"Video\"->\"Foveated encoding\".",
            "如果出现“雪花屏”闪烁，请将“预设”→“分辨率”设为“低”，并关闭“视频”→“注视点编码”。",
        ),
        (
            "Increasing \"Video\"->\"Color correction\"->\"Sharpness\" may improve the perceived image quality.",
            "提高“视频”→“色彩校正”→“锐度”可能改善画面的主观清晰度。",
        ),
        (
            "If you have problems syncing external controllers or trackers to ALVR tracking space, add one element to \"Headset\"->\"Extra OpenVR properties\", then set a custom \"Tracking system name string\".",
            "如果外部控制器或追踪器无法与 ALVR 追踪空间同步，请在“头显”→“额外 OpenVR 属性”中添加一个元素，然后设置自定义的“追踪系统名称字符串”。",
        ),
        (
            "ALVR supports hand tracking gestures (\"Presets\"->\"Hand tracking interaction\"->\"ALVR bindings\"). Check out wiki how to use them properly: https://github.com/alvr-org/ALVR/wiki/Hand-tracking-controller-bindings.",
            "ALVR 支持手势追踪（“预设”→“手势追踪交互”→“ALVR 按键绑定”）。请参阅 Wiki 了解正确用法：https://github.com/alvr-org/ALVR/wiki/Hand-tracking-controller-bindings。",
        ),
        (
            "If hand tracking gestures are annoying, you can disable them in \"Headset\"->\"Controllers\"->\"Hand tracking interaction\". Alternatively, you can enable \"Hand tracking interaction\"->\"Only touch\".",
            "如果手势追踪操作造成困扰，可在“头显”→“控制器”→“手势追踪交互”中禁用，或启用“手势追踪交互”→“仅触摸”。",
        ),
        (
            "You can fine-tune the controllers' responsiveness with \"Headset\"->\"Controllers\"->\"Prediction\".",
            "可以通过“头显”→“控制器”→“预测”微调控制器的响应速度。",
        ),
        (
            "If the visual controller/hand models do not match the physical controller's position, you can tweak the offset in \"Headset\"->\"Controllers\"->\"Left controller position/rotation offset\" (affects both controllers).",
            "如果虚拟控制器或手部模型与实体控制器位置不一致，可在“头显”→“控制器”→“左控制器位置/旋转偏移”中微调（同时影响两个控制器）。",
        ),
        (
            "When using external trackers or controllers, you should set both \"Headset\"->\"Position/Rotation recentering mode\" to \"Disabled\".",
            "使用外部追踪器或控制器时，应将“头显”中的“位置/旋转重定位模式”都设为“禁用”。",
        ),
        (
            "You can enable tilt mode. Set \"Headset\"->\"Position recentering mode\" to \"Local\" and \"Headset\"->\"Rotation recentering mode\" to \"Tilted\".",
            "可启用倾斜模式：将“头显”→“位置重定位模式”设为“本地”，并将“旋转重定位模式”设为“倾斜”。",
        ),
        (
            "If you often experience image glitching, you can trade that with stutter frames using \"Connection\"->\"Avoid video glitching\".",
            "如果经常出现画面撕裂或异常，可通过“连接”→“避免视频异常”以增加卡顿帧为代价进行改善。",
        ),
        (
            "You can run custom commands/programs at headset connection/disconnection using \"Connection\"->\"Enable on connect/disconnect script\".",
            "可通过“连接”→“启用连接/断开脚本”在头显连接或断开时运行自定义命令或程序。",
        ),
        (
            "In case you want to report a bug, to get a log file, enable \"Extra\"->\"Logging\"->\"Log to disk\". The log will be inside \"session_log.txt\".",
            "如需报告缺陷并获取日志文件，请启用“其他”→“日志”→“写入磁盘”。日志位于“session_log.txt”。",
        ),
        (
            "For hacking purposes, you can enable \"Extra\"->\"Logging\"->\"Log tracking\", \"Log button presses\" and \"Log haptics\". You can get the data using a websocket at ws://localhost:8082/api/events.",
            "用于调试时，可启用“其他”→“日志”→“记录追踪”“记录按键”和“记录触觉反馈”。可通过 ws://localhost:8082/api/events 的 WebSocket 获取数据。",
        ),
        (
            "In case you want to report a bug and share your log, you should enable \"Extra\"->\"Logging\"->\"Prefer backtrace\".",
            "如需报告缺陷并共享日志，应启用“其他”→“日志”→“优先记录回溯”。",
        ),
        (
            "You can quickly cycle through tips like this one by toggling \"Extra\"->\"Logging\"->\"Show notification tip\".",
            "切换“其他”→“日志”→“显示通知提示”可快速浏览类似提示。",
        ),
        (
            "It's handy to enable \"Extra\"->\"SteamVR Launcher\"->\"Open and close SteamVR automatically\".",
            "启用“其他”→“SteamVR 启动器”→“自动启动和关闭 SteamVR”会更方便。",
        ),
        (
            "If you want to share a video recording for reporting a bug, you can enable \"Extra\"->\"Capture\"->\"Rolling video files\" to limit the file size of the upload.",
            "如需共享视频录屏来报告缺陷，可启用“其他”→“采集”→“滚动视频文件”以限制上传文件大小。",
        ),
        ("Absolute", "绝对"),
        ("Activation delay", "激活延迟"),
        ("Adapt to framerate", "根据帧率调整"),
        ("Adapter index", "显卡适配器索引"),
        ("Adaptive", "自适应"),
        ("Adaptive quantization mode", "自适应量化模式"),
        ("Amplitude curve", "振幅曲线"),
        ("Angular velocity cutoff", "角速度截止值"),
        ("Assured forwarding", "确保转发"),
        ("Auto trust clients", "自动信任客户端"),
        ("Avoid video glitching", "避免视频异常"),
        ("Bd", "BD"),
        ("Best effort", "尽力而为"),
        ("Binary conditions", "二值条件"),
        ("Binary to scalar", "二值转标量"),
        ("Bitrate", "码率"),
        ("Blend", "混合"),
        ("Blue", "蓝色"),
        ("Body tracking", "身体追踪"),
        ("Boost", "增强"),
        ("Boot delay", "启动延迟"),
        ("Brightness", "亮度"),
        ("Buffering", "缓冲"),
        ("Buffering history weight", "缓冲历史权重"),
        ("Button mapping config", "按键映射配置"),
        ("Button mappings", "按键映射"),
        ("Button set", "按键集"),
        ("Capture", "采集"),
        ("Class", "类别"),
        ("Class selector", "类别选择器"),
        ("Click threshold", "点击阈值"),
        ("Client buffer config", "客户端缓冲区配置"),
        ("Client core", "客户端核心"),
        ("Client discovery", "客户端发现"),
        ("Client gfx", "客户端图形"),
        ("Client impl", "客户端实现"),
        ("Client log report level", "客户端日志上报级别"),
        ("Color correction", "色彩校正"),
        ("Contrast", "对比度"),
        ("Curl touch distance", "握拳触摸距离"),
        ("Curl trigger distance", "握拳扳机距离"),
        ("Custom", "自定义"),
        ("Deactivation delay", "停用延迟"),
        ("Debug", "调试"),
        ("Debug groups", "调试分组"),
        ("Destination", "目标"),
        ("Deviation", "偏差"),
        ("Device", "设备"),
        ("Devices", "设备"),
        ("Direct launch", "直接启动"),
        ("Distance threshold", "距离阈值"),
        ("Drop probability", "丢弃概率"),
        ("Dynamic", "动态"),
        ("Edge direction", "边缘方向"),
        ("Edge ratio", "边缘倍率"),
        ("Edge sharpness", "边缘锐度"),
        ("Edge threshold", "边缘阈值"),
        ("Emulated headset view resolution", "模拟头显视图分辨率"),
        ("Emulation mode", "模拟模式"),
        ("Enable intra refresh", "启用帧内刷新"),
        ("Enable on connect script", "启用连接脚本"),
        ("Enable on disconnect script", "启用断开脚本"),
        ("Enable vive tracker proxy", "启用 Vive 追踪器代理"),
        ("Enable weighted prediction", "启用加权预测"),
        ("Encoder config", "编码器配置"),
        ("Enforce server frame pacing", "强制串流端帧节奏"),
        ("Entropy coding", "熵编码"),
        ("Error", "错误"),
        ("Expedited forwarding", "加速转发"),
        ("Face tracking", "面部追踪"),
        ("Feathering", "羽化"),
        ("Filler data", "填充数据"),
        ("Float", "浮点数"),
        ("Force enable", "强制启用"),
        ("Force software decoder", "强制软件解码"),
        ("Force threshold", "按压力度阈值"),
        ("Foveated encoding", "注视点编码"),
        ("Full resolution", "全分辨率"),
        ("Gamma", "Gamma"),
        ("Github", "GitHub"),
        ("Gop length", "GOP 长度"),
        ("Green", "绿色"),
        ("Hand skeleton", "手部骨骼"),
        ("Haptics", "触觉反馈"),
        ("Height", "高度"),
        ("Hide spammy events", "隐藏频繁事件"),
        ("Hide while version", "在以下版本中隐藏"),
        ("High accuracy", "高精度"),
        ("High quality", "高质量"),
        ("History size", "历史记录长度"),
        ("Host", "主机"),
        ("Hysteresis threshold", "滞回阈值"),
        ("Image corruption fix", "图像损坏修复"),
        ("Info", "信息"),
        ("Int32", "32 位整数"),
        ("Int64", "64 位整数"),
        ("Intensity multiplier", "强度倍率"),
        ("Intra refresh count", "帧内刷新次数"),
        ("Intra refresh period", "帧内刷新周期"),
        ("Joystick deadzone", "摇杆死区"),
        ("Joystick offset horizontal", "摇杆水平偏移"),
        ("Joystick offset vertical", "摇杆垂直偏移"),
        ("Joystick range", "摇杆范围"),
        ("Latency overstep multiplier", "延迟超限倍率"),
        ("Left controller position offset", "左控制器位置偏移"),
        ("Left controller rotation offset", "左控制器旋转偏移"),
        ("Left hand tracking position offset", "左手追踪位置偏移"),
        ("Left hand tracking rotation offset", "左手追踪旋转偏移"),
        ("Level", "级别"),
        ("Linear velocity cutoff", "线速度截止值"),
        ("Local", "本地"),
        ("Local floor", "本地地面"),
        ("Log button presses", "记录按键"),
        ("Log haptics", "记录触觉反馈"),
        ("Log to disk", "写入磁盘"),
        ("Log tracking", "记录追踪"),
        ("Logging", "日志"),
        ("Lossless", "无损"),
        ("Low delay key frame scale", "低延迟关键帧缩放"),
        ("Low latency", "低延迟"),
        ("Mapping type", "映射类型"),
        ("Max", "最大"),
        ("Max level", "最大级别"),
        ("Max num ref frames", "最大参考帧数"),
        ("Max prediction ms", "最大预测时间（毫秒）"),
        ("Max queued server video frames", "串流端最大视频队列帧数"),
        ("Max saturation multiplier", "最大饱和度倍率"),
        ("Maximum", "最大"),
        ("Mediacodec extra options", "MediaCodec 额外选项"),
        ("Meta", "Meta"),
        ("Min", "最小"),
        ("Mode", "模式"),
        ("Multi pass", "多遍"),
        ("Multimodal tracking", "多模态追踪"),
        ("New version popup", "新版本弹窗"),
        ("Normal", "正常"),
        ("Notification level", "通知级别"),
        ("Off", "关"),
        ("On", "开"),
        ("Only touch", "仅触摸"),
        ("Open setup wizard", "打开设置向导"),
        ("Orientation correction", "方向校正"),
        ("P frame strategy", "P 帧策略"),
        ("P1", "P1"),
        ("P2", "P2"),
        ("P3", "P3"),
        ("P4", "P4"),
        ("P5", "P5"),
        ("P6", "P6"),
        ("P7", "P7"),
        ("Packet size", "数据包大小"),
        ("Passthrough", "透视"),
        ("Performance level", "性能级别"),
        ("Pinch touch distance", "捏合触摸距离"),
        ("Pinch trigger distance", "捏合扳机距离"),
        ("Port", "端口"),
        ("Predict", "预测"),
        ("Prefer backtrace", "优先记录回溯"),
        ("Prefer eye tracking only", "优先仅眼动追踪"),
        ("Prefer full body", "优先全身追踪"),
        ("Prefer full face tracking", "优先全脸追踪"),
        ("Prefer high fidelity", "优先高保真"),
        ("Preferred codec", "首选编解码器"),
        ("Premultiplied alpha", "预乘 Alpha"),
        ("Preproc sigma", "预处理 sigma"),
        ("Preproc tor", "预处理 tor"),
        ("Prompt calibration on start", "启动时提示校准"),
        ("Publish", "发布"),
        ("Rate control mode", "码率控制模式"),
        ("Rc average bitrate", "速率控制平均码率"),
        ("Rc buffer size", "速率控制缓冲区大小"),
        ("Rc initial delay", "速率控制初始延迟"),
        ("Rc max bitrate", "速率控制最大码率"),
        ("Recentering mode", "重定位模式"),
        ("Red", "红色"),
        ("Refresh rate", "刷新率"),
        ("Remap", "重映射"),
        ("Repeat delay", "重复延迟"),
        ("Rolling video files", "滚动视频文件"),
        ("Saturation", "饱和度"),
        ("Saturation end max", "饱和度终点最大值"),
        ("Saturation end min", "饱和度终点最小值"),
        ("Saturation multiplier", "饱和度倍率"),
        ("Saturation start max", "饱和度起点最大值"),
        ("Saturation start min", "饱和度起点最小值"),
        ("Scale", "缩放"),
        ("Serial number", "序列号"),
        ("Server buffer config", "串流端缓冲区配置"),
        ("Server core", "串流端核心"),
        ("Server gfx", "串流端图形"),
        ("Server impl", "串流端实现"),
        ("Sharpening", "锐化"),
        ("Show notification tip", "显示通知提示"),
        ("Show raw events", "显示原始事件"),
        ("Sink", "输出端"),
        ("Social presence", "社交输出"),
        ("Sockets", "套接字"),
        ("Source", "输入端"),
        ("Sources", "来源"),
        ("Spatial", "空间模式"),
        ("Stage", "舞台"),
        ("Static", "静态"),
        ("Statistics history size", "统计历史长度"),
        ("Store", "存储"),
        ("Stream port", "串流端口"),
        ("Stream protocol", "串流协议"),
        ("String", "字符串"),
        ("Super sampling", "超级采样"),
        ("Temporal", "时间模式"),
        ("Threshold", "阈值"),
        ("Tilted", "倾斜"),
        ("Touch threshold", "触摸阈值"),
        ("Tracked", "已追踪"),
        ("Tracking ref only", "仅追踪参考设备"),
        ("Transcoding view resolution", "编码视图分辨率"),
        ("Tuning preset", "调优预设"),
        ("Ultra low latency", "超低延迟"),
        ("Upscale factor", "放大倍率"),
        ("Upscaling", "放大"),
        ("Use preproc", "使用预处理"),
        ("Value", "值"),
        ("Value end max", "数值终点最大值"),
        ("Value end min", "数值终点最小值"),
        ("Value start max", "数值起点最大值"),
        ("Value start min", "数值起点最小值"),
        ("Velocities multiplier", "速度倍率"),
        ("View height", "视点高度"),
        ("Vive", "Vive"),
        ("Warning", "警告"),
        ("Web server port", "Web 服务器端口"),
        ("Width", "宽度"),
        ("Wired client autolaunch", "有线客户端自动启动"),
        ("Wired client type", "有线客户端类型"),
    ];

    ENTRIES
        .iter()
        .find_map(|(english, chinese)| (*english == key).then_some(*chinese))
}

#[cfg(test)]
mod tests {
    use super::{Language, has_translation, set_language, tr};

    #[test]
    fn detects_chinese_locales() {
        for locale in ["zh", "zh-CN", "zh-Hans", "Chinese"] {
            assert_eq!(
                Language::from_locale(Some(locale)),
                Language::Chinese,
                "locale={locale}"
            );
        }
        assert_eq!(Language::from_locale(Some("en-US")), Language::English);
    }

    #[test]
    fn translates_and_falls_back() {
        set_language(Language::Chinese);
        assert_eq!(tr("Connected"), "已连接");
        assert_eq!(
            tr("untranslated technical text"),
            "untranslated technical text"
        );

        set_language(Language::English);
        assert_eq!(tr("Connected"), "Connected");
    }

    #[test]
    fn translates_complete_schema_messages() {
        for text in [
            "Enables high motion quality boost mode.\nAllows the encoder to perform pre-analysis the motion of the video and use the information for better encoding",
            "If on_connect.bat exists alongside session.json, it will be run on headset connect. Env var ACTION will be set to `connect`.",
            "To prioritize darker pixels at the expense of potentially additional banding in midtones, set to 2.2. To allow the encoder to decide priority on its own, set to 1.0.",
        ] {
            assert!(has_translation(text), "missing translation for: {text}");
        }
    }
}
