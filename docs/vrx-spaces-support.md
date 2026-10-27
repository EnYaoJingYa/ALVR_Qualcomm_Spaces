# ALVR / ThinkReality VRX：修复与原生支持复核

日期：2026-10-27。基线：`dev`，HEAD `95e7ef11` 加本地修改。本文描述源码实现；
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
# 依赖清理后重新安装 Loader；分发打包同样要求显式提供源 AAR：
cargo xtask package-client --spaces --spaces-loader-aar build/spaces-sdk-0.23.0/libopenxr_loader.aar
```

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

- 14 项宿主机测试通过：client_openxr 8、client_core 2、system_info 3、ADB 1；
  包含配置拒绝/外部眼动允许、初始化错误唤醒、USB 自动启动机会和设备识别。
- ARM64 Release APK 和 Windows Release Streamer/Dashboard 编译通过。
- APK 签名、库清单和两种 Loader 的 SHA-256 检查通过；错误来源 AAR 被准备命令拒绝。
- PC 构建前后 `session.json` SHA-256 相同。
- 本次修复保留本地 Git 提交；未推送。本轮没有在线 VRX，未做实机验收。
- 源码中仍有待完善项：10-bit/High Profile 能力按平台静态声明；裸手与佩戴检测
  没有接入；颜色与逐帧输入改动缺少当前设备测量。这些不属于“已全部原生支持”。
