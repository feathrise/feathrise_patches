<!-- 本文件说明 FidelityFX SDK 源码修订的来源、修复范围、构建方法和发行身份，不将修订 DLL 冒充 AMD 签名原件。 -->

# FidelityFX SDK 1.1.4 Vulkan 源码修订

基础来源为 AMD FidelityFX SDK 1.1.4，官方仓库为 <https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>。本目录仅保存小补丁、完整原始输入文件 SHA256 清单及 AMD MIT 许可；SDK 源码副本、中间产物和 DLL 均位于被 Git 忽略的 `.local/fsr3-sdk-patched/`。

2026-09-09起修订运行库与构建缓存独立于Cargo的`target`目录，`cargo clean`不会删除它们。2026-09-12起，新Git链接工作树的任意FSR帧生成构建会在本地DLL与manifest同时缺失或目录为空时，自动调用当前分支脚本，从只读官方SDK在当前工作树独立生成源码副本、缓存和运行包；不再读取或复制主工作树的DLL/manifest。完整文件对直接复用并继续进行六文件身份校验，半包、异常路径或未知内容拒绝自动覆盖。CMake按`-CmakePath`、`BL5_FSR3_CMAKE`、PATH、当前工作树和Git证明的主工作树`.local/build-tools/`顺序发现；当前机器已有的固定便携归档也可在SHA-256匹配后解压到当前`.local/build-tools/`，主工作树只提供构建工具。许可文件必须保留官方原始CRLF字节，已有`-text`属性禁止Git换行转换。

修订标识：`bl5-vulkan-lifetime-ui-1`。官方签名 DLL 的 SHA256 为 `A1624CC4238FEF046F30C4D80CE3F47BE63FC5F5373F49E3EE9EDB9960F54C78`；许可证 SHA256 为 `C93D509CD69D50698796C9C3274247352736EF122A3197ACE2010F0978E21AA6`。`source-manifest.json` 锁定全部 902 个 `sdk/` 与 `ffx-api/` 输入文件，包括 SDK 自带 shader 编译器。构建前同时检查官方 DLL 的有效 AMD Authenticode 签名。

## 补丁范围

- `FrameInterpolationSwapchainVK_UiComposition.cpp` 原始第 245 行及 `FrameInterpolationSwapchainVK_DebugPacing.cpp` 第 217 行将 `lineWidth=0` 改为 Vulkan 要求的 `1.0f`，解决 `VUID-VkPipelineRasterizationStateCreateInfo-lineWidth-00749`。前者同时影响普通/预乘两条 UI 管线，后者覆盖启用调试 pacing 的入口。
- `FrameInterpolationSwapchainVK.cpp` 原始第 1484–1489 行提前创建的 `frameRenderedSemaphores` 被第 1650 行再次创建覆盖；去除第一轮创建，保留根据真实交换链图像数量创建的一轮，解决丢失句柄的 `05137` 泄漏。
- UI compositor 原有全局 GPU 对象只在设备切换时清理；交换链 `waitForPresents()` 和 `killPresenterThread()` 完成后、真实交换链与替代图像销毁前显式释放 UI 帧缓冲、视图、描述符池、管线和布局，并重置所有状态，解决 UI 对象的 `05137` 泄漏。描述符池隐式释放其所有集合，不再对空集合调用 `vkFreeDescriptorSets`。本项目限定一个设备、一个活动 FSR 交换链；SDK 原有全局 compositor 也不支持多设备并发使用。
- `copyAndPresent_presenterThread` 原始第 983 行及 `composeAndPresent_presenterThread` 第 1126 行仅在 Acquire 成功时添加 binary signal，避免失败分支 signal 无消费者的 semaphore。后者同时按实际图像索引选取 signal 与 Present 等待的 semaphore，避免固定槽零与前一个呈现竞争。timeline 等待/完成进度保留，失败不会卡住资源回收。宿主旧的严格形状 submit 回调可作为旧 DLL 的防御路径；修订源码不应再触发它。
- SDR UI 合成新增一个内部 fragment push constant：仅在场景和输出同为 BGRA8 UNORM、UI 为 BGRA8 SRGB，或相应 RGBA8 配对时启用。场景从 sRGB 编码值解码至线性，UI 由 SRGB 采样自动解码，在线性空间使用原有普通/预乘透明度公式，结果再编码后写入 UNORM 输出。其他格式路径的颜色公式保持原样，无 UI 时仍走原 SDK 图像复制路径。脚本从 GLSL 重新生成普通/预乘 PS SPIR-V 头文件，不能直接使用 SDK 随包旧头文件。
- `verifyUiDuplicateResource` 创建的内部 UI 副本仅需要 `TRANSFER_DST | SAMPLED`，不再从场景输出照搬 `STORAGE`、颜色附件及复制源用途。SRGB 图像不支持 storage 用途；此修订解决独立 UI 开启后出现的 `02251/02275`，并将该副本的 FFX 资源使用描述同步改为只读。其他交换链图像与插帧算法的用途保持原样。
- 普通成功 UI 合成/透传同样递增 `framesSentForPresentation`，原 SDK 却不发布该值的 `compositionSemaphore`，导致预热/reset 普通 UI 帧后注销或 resize 永久等待。补丁在真正完成该帧命令的原提交中补上 composition timeline signal。非插帧 Acquire 失败也递增此计数，原提交只兑现 replacement；现在同一 game 提交在原等待条件之后兑现 composition/present，并同步 `lastPresentSemaphoreValue`，避免 quiesce 永等或恢复时重复 signal 同值。失败仍不提交 binary signal、不虚构实际 Present、不增加生成帧、不移除等待；调用方最多 5 个 signal，加上 `VkCommands::execute` 内部命令回收信号后最多 6 个，仍满足 SDK 固定容量 6。
- `presentSemaphore` 由 GPU 提交 signal，可能早于 CPU 的 `vkQueuePresentKHR` 返回；原 `waitForPresents()` 随后调用 QueueWaitIdle/DeviceWaitIdle，仍可能与 presenter 线程争用队列。修订使用 SDK 既有 `scheduledFrameCriticalSection` 保护独立 CPU 完成计数和 Win32 条件变量：两个 presenter 在线程内整批 submit/Present 返回后发布末帧值，普通成功/Acquire 失败则在同步分支发布。waitForPresents 在 GPU 等待后、任何队列/设备 idle 前以谓词循环等待 CPU 完成；条件等待原子释放调度锁，worker 不会因该锁被阻止完成最后一批。每个交换链初始化计数为零，不新增 GPU 信号，不靠 sleep，不提前杀线程或丢弃最后一帧。
- `presentInterpolated` 原本持有 `scheduledFrameCriticalSection` 记录并提交游戏队列合成，Presenter 取任务及发布 CPU 完成也需要这把锁。修订把该锁缩小到最终 `resetTimer` 与完整 `scheduledInterpolations` 发布；资源、计数和合成仍受外层交换链 `criticalSection` 保护，全部合成提交返回后才发布任务，随后解锁并触发原事件。这样驱动提交在 CPU 阻塞时不会同时阻止 Presenter 获取调度锁；任何 GPU 等待、信号、队列或 pacing 参数都不改变。这修复了已确认的锁范围风险，但尚未通过卡死现场栈证实它就是基础限帧由低于自然上限切到高于上限或 Off 时无响应的根因。

公开 FFX API 与 DLL 导出名保持一致；CPU 呈现完成计数及条件变量只扩展由该 DLL 自己创建和管理的 SDK 内部状态，宿主不读取其布局。补丁不使用 IAT hook、运行时二进制改写或宿主私有布局访问，也不隐藏 Vulkan 验证错误。

## 构建

需要 PowerShell 7、Git、CMake >= 3.23、Visual Studio 2022 C++ x64 工具链及 Vulkan SDK >= 1.3.250。`FFX_SC.exe` 与 `glslangValidator.exe` 使用经过输入清单校验的 SDK 自带版本。

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/构建FSR修订运行库.ps1 `
  -SdkRoot D:/SDK/FidelityFX-SDK-v1.1.4 `
  -VulkanSdk C:/VulkanSDK/1.4.357.0
```

脚本只复制、修订和构建当前工作树 `.local/fsr3-sdk-patched/` 内的源码，不修改传入的 SDK 原件。未显式传入`-CmakePath`时使用上述自动发现；需要固定其他安装时可设置`BL5_FSR3_CMAKE`。Cargo自动入口按环境PATH、PowerShell 7标准安装目录与Codex固定依赖目录寻找`pwsh.exe`，非标准安装可通过`BL5_FSR3_PWSH`传入完整路径。首次构建包括 FSR2、FSR3 upscale、optical flow、frame interpolation 的官方 shader permutations。默认两个并行构建工程，可通过 `-Jobs 1` 进一步降低并行度。

成功后一次发布 `.local/fsr3-sdk-patched/runtime/amd_fidelityfx_vk.dll`、`manifest.json`与`LICENSE.txt`；宿主使用的MIT许可、补丁、源码清单及README继续从当前工作树受版本控制目录组成完整六文件身份链。manifest schema 1 记录 SDK 版本、修订号、官方 DLL/许可/补丁/完整源码清单/产物 SHA256、产物尺寸、实际 Authenticode 状态、CMake 版本与构建配置。宿主构建应同时核验仓库中的补丁/源码清单并将 DLL 身份固化进程序；运行与发行不能只相信 DLL 旁边可被替换的 JSON。

本 DLL 是项目从 AMD MIT 许可源码构建的修订版，**没有 AMD Authenticode 签名**。官方签名原件继续作为来源锚点，修订 DLL 使用源码、补丁及产物哈希链。不同编译器或重新链接可能改变产物哈希，不承诺跨构建位级一致；重建之后应重新完成宿主构建、发行身份锁定与真实 Vulkan 验证。

## 验收边界

构建成功仅证明源码及 shader 可以编译。还需由宿主启用真实 Vulkan 验证层运行 FSR-G，覆盖无 UI、独立 UI、连续呈现、窗口 resize 与退出；要求 `00749`、`05137`、binary semaphore `00067` 均为零，同时验证生成帧持续增长和 UI 透明边缘颜色。清除验证日志、过滤错误或只观察 FPS 都不能替代此验收。

UI 生命周期回归必须覆盖“新交换链预热普通 UI 帧 → 下一帧 UI 管线 Pending/clear → 继续普通呈现”和“普通 UI 帧 → UI 尺寸变化 → 重建”，以及非插帧 Acquire 返回 out-of-date 后立即 quiesce。检查操作在限定时间内返回、正常退出、所有 VUID/SYNC 为零，恢复后连续窗口仍有真实生成/Present 证据。仅稳定开启 UI 的固定尺寸测试无法覆盖上述计数缺口；纯粹复制计数公式的测试也不能证明 GPU 提交顺序，最终以真实 resize/失败路径验证为准。

退出回归还必须覆盖后台 presenter 仍在最后一次 `vkQueuePresentKHR` 内时请求关闭，等待其 CPU 完成后再进入 QueueWaitIdle/DeviceWaitIdle。检查全部 `UNASSIGNED-Threading`、VUID 和 SYNC 错误，不能只检查 GPU semaphore；曾经4K稳定阶段全部2×但退出报队列竞争的结果属于整体失败。需要使用本次 CPU 完成修订产物重新执行六次resize、4K与不同超分组合，确认最后一帧被正常处理且退出无竞争。

调度锁范围回归需在相同场景与画质下分段保持基础限帧 40、50、70、91、Off 等设置，覆盖从低于实测自然基础帧率切到高于上限的满负荷边界，并检查有限时间内继续生成、实际呈现和退出。一次自动重放未卡死不能证明用户问题已经解决；若再次无响应，应保留游戏线程及 Presenter 的等待链/调用栈，区分驱动提交、调度锁、GPU timeline 和窗口消息等待，不能过滤失败统计或用调整 pacing 参数替代定位。
