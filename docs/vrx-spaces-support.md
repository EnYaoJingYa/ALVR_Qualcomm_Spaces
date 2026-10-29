# ALVR / ThinkReality VRX：修复与原生支持复核

日期：2026-10-27。基线：`dev`，HEAD `e2925295` 加本地修复。本文描述源码实现；
构建成功不等于设备验收。本轮没有在线 VRX，未安装或启动头显应用。

## 结论

**没有做到“所有 ALVR 功能均有 Qualcomm 原生实现”。** 基础 XR 体验使用系统
Qualcomm Spaces Runtime，通过标准 OpenXR 和 Android 原生 API 实现；
PC 编码、网络协议和 ALVR 图像处理不需要改写为 QCOM 扩展。
原生眼/面部/身体追踪、设备侧 foveation 和透视等仍未接入 VRX 后端。

`XR_FB_display_refresh_rate` 的前缀不表示调用 Meta Runtime；该扩展也是
Qualcomm 官方 Base Runtime Feature 使用的接口。Touch interaction profile
同理由 Spaces 官方示例明确采用，不依赖 Quest Runtime。

## 本轮修复

1. VRX 的 `EXITING` 结束 XR 体验并结束 Activity；`LOSS_PENDING` 保留会话重建语义。
   Android `Destroy` 单独通知渲染循环退出。C 入口不接管宿主 Activity。
2. PC 有线自动启动对 VRX 在同一次连接观察周期只自动拉起一次，手动运行也消耗该机会。
   用户退出后等待手动打开；PC 观察到设备离线、自动启动关闭，或 Streamer 重启后可重置。
   未观察到的快速拔插/开关变化不保证重置。其他设备保留原自动启动策略。
3. Qualcomm Loader 从官方 SDK 0.23.0 AAR 的 **JNI ARM64** 项提取，独立命名
   `libopenxr_loader_spaces.so`；输入 AAR 和输出 SO 均校验固定 SHA-256。
   通用 Khronos 1.1.36 Loader 使用原文件名，两者不互相覆盖。
4. VRX 的启动及实时 CPU/GPU 性能请求统一检查结果；实际渲染线程注册为
   `XR_ANDROID_THREAD_TYPE_RENDERER_WORKER_KHR`，注册失败直接报错。
5. 串流开始及实时配置应用前拒绝未实现的透视、FB 合成器后处理、设备侧 foveation、
   原生眼/面部/身体追踪和 Meta 多模态/分离手柄请求。关闭的配置不误报。
   外部 OSC 眼动与头显原生眼动分开处理；仅外部 eye-only 模式允许。
6. 固定注视点编码恢复 ALVR GLES shader 路径的能力声明，不再绑定眼动扩展。
   没有强制开启本地会话中已关闭的编码选项。
7. VRX 默认 MediaCodec 初始化失败后不再自动重试软件解码；HDR 必须获得
   `GL_RGBA16F` 交换链，不能改用 SDR。VRX 的 `xrEndFrame` 失败直接报错，
   不再次提交替代帧。显式选择软件解码仍属于用户设置，不能算硬解支持。
8. 修复 Android 解码初始化失败时等待方永不唤醒的问题。成功/失败均保存结果、
   唤醒创建方，并处理通知早于等待及虚假唤醒。这项同步修复适用于共用 Android
   解码器；其他平台的自动软件重试策略保持原有逻辑。

本轮以前已有、仍保留的修复：采用 Runtime 推荐/最大眼缓冲尺寸；移除无实际图像
跟踪用途的 tracking optimization 依赖；请求刷新率后查询实际值，不将异步切换
误判为失败；补充官方 Manifest 权限和包可见性。

## 功能矩阵

“已接入”表示可追踪到实际实现，仍需本轮 APK 的 VRX 实测。

| 功能 | 实际实现 | 复核结果 |
| --- | --- | --- |
| 设备识别 | Lenovo + VRX，`Platform::ThinkRealityVRX` | 已接入，有识别单元测试 |
| Loader / Runtime | SDK 官方 Loader → 系统 Spaces；API 1.0.34 | 已接入，独立文件及固定哈希；新 Loader 未实机验收 |
| 双眼显示 | OpenXR GLES session、双交换链、projection layer | 已接入；历史曾完成 H.264 串流 |
| 眼缓冲尺寸 | Runtime 枚举推荐值/最大值 | 已接入，未硬编码物理面板尺寸 |
| 头部 6DoF / IPD / FOV | OpenXR locate/relate/views | 已接入；当前版本需实测精度和重定位 |
| 左右手柄 6DoF、aim/grip | Touch profile + OpenXR actions/spaces | 与官方 profile 一致；实体姿态需复测 |
| 按键、扳机、握把、摇杆 | OpenXR boolean/float actions | 已接入；逐键、触摸通道及系统保留键需实测 |
| 左右震动 | OpenXR HapticVibration + VIBRATE | 已接入；幅度/频率/持续时间需实测 |
| 刷新率 | 枚举、请求、查询 `XR_FB_display_refresh_rate` | 已接入；请求成功不保证同步完成切换 |
| CPU/GPU hint | `XR_EXT_performance_settings` | 已接入并检查结果；不同等级需实测 |
| 渲染线程优先级提示 | `XR_KHR_android_thread_settings` | 已接入并检查结果；实际收益待测 |
| H.264 / HEVC | Android NDK MediaCodec、ImageReader、AHardwareBuffer | 原生 API 路径已接入；H.264 有历史记录，HEVC 需验收 |
| 硬件解码器能力 | 默认 MediaCodec 按 MIME 选择 | **未枚举并验证实际 codec 的硬件加速属性**；不能声称所有请求均由 Qualcomm 硬解 |
| 10-bit / High Profile | 目前按已知平台声明支持 | **尚未按当前设备 MediaCodec profile/level、尺寸和帧率实查** |
| AV1 | VRX 不声明 AV1 能力 | 未列为 VRX 支持项 |
| HDR | GLES 浮点交换链，缺失即报错 | 路径存在；显示、解码和色彩端到端未验收 |
| 固定注视点编码 | ALVR 自身 shader 恢复编码图像 | 已接入，与原生眼动/设备侧 foveation 独立；未实机验收 |
| 图像缩放/锐化 shader | ALVR GLES/wgpu 图像处理 | 通用路径存在；不同配置性能/画质待测 |
| 设备侧 foveation | 现有实现依赖 FB swapchain 扩展 | **无 VRX 原生实现，显式请求时报错** |
| 合成器后处理 | 现有实现依赖 FB composition-layer 扩展 | **无 VRX 实现，显式请求时报错**；与 ALVR 自身 shader 分开 |
| 音频/麦克风 | CPAL 0.16 的 Android AAudio 后端 | 通用原生 API 路径存在；路由、权限、延迟需实测 |
| USB 有线 / Wi-Fi | ADB port forward / ALVR sockets | 通用传输已接入；不是 Qualcomm 专用 USB 协议 |
| 重投影 | OpenXR Runtime 合成器；未启用 YVR 自定义重投影 | 已接入；延迟和掉帧下的效果需测量 |
| 裸手骨骼跟踪 | 当前 VRX 扩展集未启用 `XR_EXT_hand_tracking` | **未接入 VRX**；SteamVR 手柄手指模拟不等于原生裸手追踪 |
| 原生眼/面部/身体追踪 | 未接入相应 Spaces 数据源 | **未实现，相关显式请求时报错** |
| 外部 OSC 眼动 | PC 接收外部数据 | 可保留；不能算头显原生眼动支持 |
| 多模态/分离手柄 | 现有实现为 Meta 专用扩展 | **无 VRX 实现，请求时报错** |
| 透视/MR/空间锚点等 | 未接入 Spaces 对应功能 | **未实现**；透视设置明确拒绝 |
| 佩戴状态 | 当前 VRX 未启用 user-presence 扩展 | **没有完整原生佩戴检测**，不能宣称摘戴行为已验收 |
| 中文界面 | PC 与客户端共用 ALVR localization | 已有实现；本轮保留 |

## 官方依据

- Qualcomm 官方 VRMR SDK 0.23.0 UE5.3 插件：
  `SpacesXRControllerInput.cpp` 的 Touch profile/haptics；
  `SpacesRuntimeModule.cpp` 的 Loader 初始化；
  `SpacesRuntime_UPL.xml` 的官方 AAR 打包；`SpacesRefreshRateExtension.cpp` 的刷新率调用。
  工作区源包：`SnapdragonSpaces/official-sdk/unreal/VRMR_Snapdragon_Spaces_SDK_0_23_0_for_UE.zip`。
- Qualcomm Unity SDK 1.0.4：`BaseRuntimeFeature*.cs`、`SpacesSetting.cs`、
  Core Sample `SampleController.Quit`、Fusion Sample `CanvasControllerCompanion.Quit`。
  线程提示来自 `BaseRuntimeFeature.AndroidThreadSettings.cs` 与 `.RenderEvents.cs`。
  新版示例提供用法依据，不自动证明所有功能可在旧固件运行。
- Khronos OpenXR 1.0 规范：EXITING 与 LOSS_PENDING、Android thread settings、
  display refresh rate、performance settings。工作区文本：
  `SnapdragonSpaces/official-docs/text/khronos-openxr-1.0-specification.txt`。
- MediaCodec 失败处理和固定注视点 shader 是对 ALVR 本身实现的复核；
  Qualcomm Core Samples 没有覆盖 ALVR 视频解码与颜色链路。

## 可复现的 Loader 准备与构建

从官方 VRMR SDK 0.23.0 ZIP 的 `Package for UE5_3` 中打开插件 ZIP，取出
`ThirdParty` 下的 `libopenxr_loader.aar`，放在 `deps/android_openxr` 以外。
本地本轮输入位于 `build/spaces-sdk-0.23.0/libopenxr_loader.aar`。
源 AAR 不进入 Git，不把系统 Runtime、Services APK、Lenovo 私有库打进 ALVR。

```powershell
cargo xtask prepare-spaces --spaces-loader-aar build/spaces-sdk-0.23.0/libopenxr_loader.aar
cargo xtask build-client --release --spaces
# build-client 必须已准备正确的 Loader；不再需要 --spaces 才检查。
# 依赖清理后重新安装 Loader；默认 GitHub 分发打包要求显式提供源 AAR：
cargo xtask package-client --spaces --spaces-loader-aar build/spaces-sdk-0.23.0/libopenxr_loader.aar
# 在现有安装目录内重建桌面 Streamer 时保留用户 session.json：
cargo xtask build-streamer --keep-config
```

GitHub 的 APK 构建和发布工作流通过仓库 Secret `SPACES_LOADER_AAR_URL` 获取
上述官方源 AAR。该值应是构建机可访问的授权 HTTPS 下载地址；下载后先校验固定
AAR 哈希，xtask 再校验 JNI ARM64 库。Qualcomm SDK 下载要求登录，不使用匿名
登录页或其他版本替代，也不把源 AAR 提交到仓库。Secret 缺失、下载失败或哈希
不符都会阻止 APK 产出；来自 fork 的 PR 默认没有此 Secret，需由维护者在可信
分支执行产物构建。Rust Android 检查工作流只运行 `cargo apk check`，不生成缺库 APK。

2026-10-27 复核修复：关闭自动启动期间即使仍观察到 VRX 进程，也不消耗下次启动
机会。VSync 输入线程保留已消费序号，渲染通知早于等待也能处理；测试用超时接收
并清理线程，不再依赖 10 ms 睡眠和无限等待。

VRX 的有线串流帧提交使用 `xrWaitFrame` 返回的单调递增
`predictedDisplayTime` 作为 `xrEndFrame::displayTime`。此前传入可能重复的视频
时间戳会被高通合成器判定为 `Duplicate display time`，随后出现
`Too many frames are queued` 和明显卡顿。该规则来自 OpenXR 1.0 规范
“Frame Submission”和“xrWaitFrame”定义；Quest/Pico 既有提交时间逻辑不受影响。

有线连接也必须保持严格。此前 ADB loopback 控制连接短暂失败后，服务端会继续
执行 mDNS 发现并可能改连 Wi-Fi；现在只要 `client.wired` 已就绪，失败就记录错误并
重试 USB/ADB 路径，不再静默切换网络。不要把 `client.wired / 127.0.0.1` 与
`*.client.local. / 192.168.x.x` 混为同一条链路。

VRX 还显式请求公开扩展 `XR_QCOM_tracking_optimization_settings` 的
`DOMAIN_ALL + HIGH_POWER_PRIORIZATION`，以降低头部/控制器跟踪延迟；扩展缺失或
调用失败会直接报错。

### 35–40 ms 延迟边界

当前 Qualcomm Spaces OpenXR 路径不能把端到端总延迟压到 35–40 ms。实测
`vsync_queue` 单独约 `35.6 ms`，还未计入游戏时间、编码、USB/ADB、解码、解码缓冲
和客户端合成。Runtime 枚举的 QCOM 扩展中没有 VSync 队列或 late-latch 深度控制。

逆向当前 `VrStreamSpace.apk` 后确认：

- 当前应用使用 OpenXR；`libInterceptOpenXR.so` 只转发
  `xrWaitFrame/xrBeginFrame/xrEndFrame` 并插入本图层，没有修改显示时间。
- APK 中遗留的 `GSXRSetVSyncCount` 是旧 GSXR 路径；当前 APK 不包含
  `libgsxrplugin.so`，不能视为可调用接口。
- 公开 OpenXR 扩展只有 `XR_QCOM_tracking_optimization_settings` 可用于跟踪
  优化，没有公开的低延迟合成器队列控制。

因此，在不 root、不打包厂商私有库的前提下，35–40 ms 目标需要更换帧提交后端
（例如取得 Qualcomm/Lenovo 正式授权的原生 QVR/GSXR 或 Vulkan late-latching 接口），
仅修改 ALVR 参数无法达到这一下限。ALVR 侧仍应保持：NVENC P1/UltraLowLatency、
关闭多遍和 AQ、严格有线连接、有限解码缓冲，以及高性能跟踪提示。

| 依赖 | SHA-256 |
| --- | --- |
| 官方 0.23.0 AAR | `BE239F71F8ABDC88730218F2A23988DB5E47355AABE85FA65699888F4A2C4132` |
| JNI ARM64 Loader | `934722B432BD85C07C1CF07FBA0CD3C0E597BAF58F7FE7B64CF669BFD401F401` |
| 通用 Khronos 1.1.36 AAR | `7BB8EF37F7FE138233998C64529E0808EE33F588986A6E54A9015ABC40B44BE7` |
| 通用 Khronos ARM64 Loader | `6B9AEB6EA2E429BAE91328CC85B0E19947CC77CA2F4948A763A4A41C23511DD6` |

`spaces.version=0.23.0` 记录本轮 SDK 依赖；`minApiLevel=0.22.0` 是功能最低要求，
OpenXR API 1.0.34 是设备 API 基线。这三个版本号含义不同，不做失败后降级重试。

## 尚需设备验收

优先验证退出/重连、首次 Loader 启动、左右手柄逐键与震动、H.264/HEVC，
再验证 90 Hz 帧时序、实际硬件 decoder 名称/能力、HDR/10-bit 与灰阶色阶。
先前关于低延迟和色彩的本地改动保留，但不是官方样例已经证明的优化：
当前渲染线程等待视频帧仍有 500 µs 轮询，逐帧输入通知的收益也未测量。
不支持选项目前沿用客户端的 fatal error 路径，记录具体功能/API 失败后终止，
尚未增加可在头显中恢复设置的交互界面。

## 验证记录

本轮本地修复（基于 `e2925295`，尚未提交）：

- 最新的 client_openxr 10 项宿主机测试通过；server_core 编译及测试目标通过。
  新增自动启动关闭期间观察进程、VSync 提前通知、VRX 显示时间和扩展集回归测试。
- 新版 xtask 编译通过；缺少源 AAR、错误源 AAR、缺少 Spaces Loader 三项构建拒绝
  检查通过，失败前后现有 Loader 与 APK 的 SHA-256 保持一致。
- ARM64 Release APK 编译和 v3 签名验证通过，两种 Loader 的 SHA-256 与固定依赖
  一致；产物 `build/alvr_client_android/alvr_client_android.apk`，SHA-256 为
  `2A7C2FA46DA8E536AB27B539F4175747858BB6E4841084C71F7DBD9D97CB09DF`。
- CI 下载动作的缺少地址、正确 AAR、错误 AAR 三项本地模拟检查通过；
  YAML 和 Bash 语法检查通过。未配置远端 Secret，也未运行 GitHub Actions。
- 修复版已安装并启动在 `PS0012P0`；PC 显示 `Streaming`，SurfaceFlinger 报告
  90 FPS。观察期间 `Duplicate display time`、`Too many frames are queued` 和
  `xrEndFrame` 错误均为 0；主观画面流畅度与长时间稳定性仍需用户体验确认。
- 有线会话曾实际落到 `8801.client.local. / 192.168.1.7`（Wi-Fi），而
  `client.wired / 127.0.0.1` 断开。禁止静默回退后，只有 wired loopback 建立
  Streaming，Wi-Fi 客户端保持 Disconnected。

`e2925295` 提交的验证记录：

- 14 项宿主机测试通过：client_openxr 8、client_core 2、system_info 3、ADB 1；
  包含配置拒绝/外部眼动允许、初始化错误唤醒、USB 自动启动机会和设备识别。
- ARM64 Release APK 和 Windows Release Streamer/Dashboard 编译通过。
- APK 签名、库清单和两种 Loader 的 SHA-256 检查通过；错误来源 AAR 被准备命令拒绝。
- PC 构建前后 `session.json` SHA-256 相同。
- 该轮修复已保留本地 Git 提交；未推送，当时没有在线 VRX，未做实机验收。
- 源码中仍有待完善项：10-bit/High Profile 能力按平台静态声明；裸手与佩戴检测
  没有接入；颜色与逐帧输入改动缺少当前设备测量。这些不属于“已全部原生支持”。

### 2026-10-29 黑屏与有线重连修复

- VRX 分支将解码轮询截止时间设为“不等待”，但渲染循环只在截止时间存在时才调用
  `get_frame()`，导致解码队列完全不被消费。现改为所有平台每帧至少执行一次
  非阻塞取帧；VRX 不额外等待，其他平台仍按原截止时间补轮询。
- VRX Runtime 可能在头显已经佩戴时持续报告 `UserPresence=false`。VRX 现在忽略
  该错误事件以保持 SteamVR HMD active；Pico、Quest 等平台保持原行为。
- 有线客户端已有活动会话时，握手线程不再并行尝试残留 mDNS/Wi-Fi 客户端。
- `alvr_client_openxr` 12 项宿主机测试通过，Release APK 和 Windows Streamer
  构建通过。实机有线会话达到 `Streaming`，Dashboard 显示约 89 Client FPS、
  590 packets/s，VRX 截屏已恢复双眼 SteamVR 画面。
- 本次 APK SHA-256：`4DEB07A02791DB5144F0472690CB1BA8967575CEBA158115DDC0C0B570E4D584`。

### 2026-10-29 有线高延迟与重复编码修复

用户截图中的 35 Mbps 配置对应约 256 Mbps 实际视频吞吐，且 Streamer 显示
603 FPS、Client 显示 89 FPS。根因是 Windows SteamVR 驱动的重复 `Present`
路径把同一 tracking timestamp 再次送进 NVENC。每帧在 2144×2144、低延迟
H.264 下存在实际最低帧大小，因此重复编码把传输量推高到配置值数倍，并造成
约 28 ms 的 USB/ADB 传输段。

Windows 驱动现在记录最后一次成功提交的非零 target timestamp，并跳过其重复
`Present`；零时间戳仍允许重试。修复后同一 35 Mbps 会话的 Dashboard 实测
码率约 34.3 Mbps。新增直接统计项用于显示游戏渲染、Streamer 合成、帧缓冲、
客户端合成和 VSync 队列，并修正多 NAL 帧的总字节统计和重复 Present 的
Streamer FPS 统计。

一次 600 帧统计窗口的平均分段如下（2.0 帧缓冲）：

| 分段 | 平均 | 范围 |
| --- | ---: | ---: |
| 总延迟 | 61.23 ms | 48.01–70.88 ms |
| 游戏渲染 | 1.65 ms | 0.71–3.75 ms |
| Streamer 合成 | 0.20 ms | 0.12–2.43 ms |
| 编码 | 2.47 ms | 1.37–3.86 ms |
| USB/ADB 传输 | 3.82 ms | 1.67–15.15 ms |
| 解码 | 3.82 ms | 1.88–9.88 ms |
| 帧缓冲 | 11.35 ms | 0.11–23.49 ms |
| 客户端合成 | 2.55 ms | 1.04–5.71 ms |
| VSync 队列 | 35.38 ms | 31.94–37.32 ms |

瓶颈已经不在编码、USB 或解码，而是 Qualcomm Spaces/OpenXR 客户端的
VSync 提交队列。1.0 帧缓冲测试进一步把平均总延迟降到约 55.7 ms、帧缓冲
降到 6.3 ms，但帧率波动更明显；1.5 帧缓冲平均总延迟约 58.5 ms、帧缓冲约
8.5 ms、客户端平均约 88 FPS，已作为最终折中保留。若目标仍是 35–40 ms，
剩余约 35 ms 的 VSync 队列需要正式授权的原生 late-latching/低延迟合成接口，
单纯降低 ALVR 码率或更换 USB 线不能消除。

同时修正 `Disconnecting` 状态的恢复：该状态不再被当作永久活动连接，也不再
阻止有线握手；新的有线连接可从残留 `Disconnecting` 状态恢复为 `Streaming`。
