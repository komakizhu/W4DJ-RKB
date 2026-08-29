# W4DJ RKB 当前项目状态

最后核对：2026-08-25（Asia/Shanghai）

## 2026-08-25 文件名路由修复与验收

2026-08-25 的产品规则修正已实施：输出名优先遵循用户选择的 `FilenameRule`，而不是固定使用 `PreserveSource`。`TitleArtist`/`ArtistTitle` 使用解析出的源身份；`Original` 才使用源文件 basename。NUL 和 ASCII `/` 只在文件名边界分别转换为 `, ` 和全角 `／`；元数据、封面和数据库匹配继续使用原始身份。

本轮验证：Rust `cargo test --all` 全部通过；文件名规则聚焦测试 117 项、预览 20 项；前端 Vitest 12 个文件、191 项通过；Vite 生产构建、Rust fmt、Tauri `cargo check`、Tauri all-targets Clippy（`-A dead_code -D warnings`）和 `git diff --check` 通过。五首真实 NUL 歌手 MP3 已复制到隔离临时目录完成扫描、真实 FFmpeg 转换、目标文件名和源 Title/Artist/封面回读，用户目录、现有输出和数据库未被修改。10 首 ASCII `/` 标题的批量现场回读、ExifTool 全字段报告、Windows 和 GUI 人工验收仍未宣称完成。

## 基线

- 当前版本：`3.2.0-beta.3`（对外显示为 W4DJ 3.2.0 beta-3；根 Cargo、Tauri Cargo、Tauri 配置和前端 package 已同步）。
- 当前开发阶段：`3.2.0 beta-3`。
- 当前分支：`codex/v3.0.2`，HEAD `236e671 ci: refresh npm lockfile for tensorflow`；远端跟踪 `origin/codex/v3.0.2`。
- 标签：当前 HEAD 有 `v3.2.0-beta.2` 标签；工作树中的产品版本定义为用户明确要求的 `3.2.0-beta.3`。
- 工作树：不干净。既有 tracked 修改和大量 untracked 文件；本次交接还会新增 `AGENTS.md`、本文件、`docs/handoff.md`。没有执行清理、提交、推送或分支切换。

## 当前架构

Tauri 2 桌面壳调用共享 Rust 音频转换库；前端 TypeScript/Vite 负责两个任务槽、转换/扫描/增强状态、教程和歌曲库 Dashboard。增强分析的浏览器解码/重采样保留在主线程，Essentia.js/WASM 基础分析、MusiCNN 帧计算和 TensorFlow.js 模型推理由 Vite 模块 Worker 执行，音频与模型权重通过可转移缓冲传递。Rust 层使用 FFmpeg sidecar、ID3/FLAC/RIFF 元数据处理、Essentia 分析缓存、独立扫描缓存、转换历史和本地运行会话记录，以及 `<app-data>/w4dj.sqlite3` 输出歌曲库。`library-dashboard.sqlite3` 只作为兼容网易云元数据暂存库；网易云 SQLite 只在转换阶段读取用于元数据处理，不再作为 Dashboard 歌曲库来源，原始数据库不被修改。

## 已完成并有自动化验证的能力

- 两个任务槽的来源/输出选择、拖放、输出目录打开、扫描后转换/直接转换、普通/增强、兼容/无损和 WAV/AIFF 原地动画。
- 转换历史、失败/取消状态、本地运行会话记录、分析缓存和独立扫描缓存的基础能力；运行会话保留在本机供诊断，转换历史中的“导出错误报告”由用户手动选择路径后生成 UTF-8 文本，并回写导出路径，新的转换不自动生成报告文件。
- 转换事务和临时输出、元数据/封面写入路径、MP3/FLAC/WAV/AIFF 的基础处理，以及 WAV 的 RIFF INFO 兼容处理。
- 网易云本地记录读取和元数据合并仍供转换链路使用；W4DJ 私有 SQLite 的 schema、分析结果投影、恢复和参数化查询已接入。
- 歌曲库 Dashboard：只查询 W4DJ 输出歌曲库，支持总数/可用/失效/未分析/失败/完成统计、搜索、基础筛选、排序、分页、详情、Essentia Genre、歌词查看/复制/下载和封面加载接口；已移除“取消更新”“选择数据库”和旧的“分析本地歌曲”入口，改为“重新分析当前输出”。
- 独立输出歌曲库：`src/w4dj_library.rs` 创建并恢复 `w4dj.sqlite3`，从成功转换历史做一次性导入；安全提交后的每个输出立即登记，保存实测格式/大小/时长/码率、来源路径、任务槽和输出根目录。任务槽首次在新根目录成功产出后才切换，旧根目录记录标记为 `outOfScope`。
- 歌曲库维护后台：`find_invalid_library_tracks` / `cancel_invalid_library_scan` 通过 `library-invalid-scan-progress` 扫描所有已登记路径并协作取消；“清除所有失效文件”只事务删除 W4DJ SQLite 的失效记录及关联分析，不删除音频、网易云数据库或历史缓存。
- 歌曲库维护操作：歌曲行仅在右键菜单中提供“重新定位文件”和“移除记录”；前者更新 W4DJ SQLite 的本地文件绑定并保留分析结果，后者只删除 W4DJ SQLite 记录。搜索栏右侧提供需勾选确认的“清除所有失效文件”，不会删除本地音乐、网易云数据库或分析缓存。
- 增强分析相关的基础分析、Drop LUFS 选择逻辑、情绪标签过滤及报告/缓存字段；分析期间的重计算已移入专用 Worker，支持按歌曲/阶段进度、旧任务消息过滤和立即终止取消；分析进度携带 `slotIndex` 并按来源任务槽分组，任务 2 不再覆盖任务 1 的进度条；模型缺失不应阻止普通转换。
- 增强分析稳定性修复已完成：MusiCNN 使用显式逐帧释放、每 32 帧让出 Worker 事件循环并发送心跳；每首歌曲独立创建/销毁 Worker，超时按 `min(15 分钟, max(5 分钟, 时长秒数×3+60秒))` 终止当前歌曲并继续下一首。取消保留已完成结果，当前歌曲不写成功结果；历史摘要和手动错误报告现在分别列出转换状态、分析总览、逐曲 pending/running/completed/failed/timeout/cancelled、开始/结束时间、Worker、阶段和耗时。
- TensorFlow.js 与 MusiCNN、五个既有 Mood 头、人声/器乐头、emoMusic、MuSe、MIREX 已随 App 离线打包；启动时自动补齐缺失/损坏副本，不依赖运行时模型下载。三个新增情绪 head 使用本地官方 ONNX 导出并通过严格输入/输出/权重校验，不生成占位模型。Discogs EffNet 共享 embedding、`genre_discogs400` head 和 Mood/Theme、Approachability、Instrumentation、Timbre、Discogs Danceability 五个 head 也已离线转换并内置：embedding 使用 `[64,128,96]` 输入和 1280 维输出，Genre head 返回官方 400 类标签，五个 head 共享同一 embedding；浏览器 GraphModel 不支持的 `PartitionedCall` 已在离线准备阶段按函数库等价展开，未改变权重或推理语义。MusiCNN 的 50 个 MSD 标签写入 `style`，Discogs Genre 结果写入正式 `genre`，五个新增结果保留在独立 `highLevel.discogsEffnet` 命名空间，不回退成 Style。
- 工作区新增独立 `tools/emotion-evaluation/` 四模型主观验收页面：Rust/CLI 从 `w4dj.sqlite3` 的可用输出导出带随机 seed、相对路径、Drop/峰值能量 10 秒片段和四套模型状态的 manifest；浏览器先收集不看模型的主观情绪，再匿名呈现 A/B/C/D，结果保存到 IndexedDB 并由用户手动导出 JSON/CSV。它不运行模型、不修改 Dashboard、歌曲库、分析缓存或转换历史。
- Essentia 模型校验、恢复和导入命令仍保留在 Rust 后端，作为兼容/维护能力，但不再作为普通用户界面入口；模型文件拖入窗口也不再进入导入流程，歌曲/文件夹继续走原任务槽逻辑。设置区只保留增强分析缓存和扫描缓存清理按钮。
- 使用帮助、第五步教程入口、术语统一、模式切换防闪烁、WAV/AIFF 浮现动画和清理分析/扫描缓存入口。
- 任务 1 来源标题右侧提供“扫描本地网易云文件夹”显式入口；点击后先调用 `locateNeteaseLibrary(true)`，成功返回的 `musicFolder` 通过现有任务 1 来源流程保存，不建立 Dashboard 数据。自动发现失败时按钮才变为“手动选择文件夹”兜底；`netease-discovery-progress` 映射到任务 1 进度条，高频事件只做局部 DOM 更新。启动、教程完成和 WebView reload 都不会自动定位网易云数据库，页面 reload 也会跳过已完成教程。

## 已实现但尚未完全验证

- 网易云数据库定位和读取仍供转换阶段的元数据匹配链路使用；当前 Dashboard 不调用定位、刷新或手动数据库命令。尚未用用户提供的真实数据库完成 `28712318`、`3409113568` 和 `meta` 封面全链路验收。
- 旧 Dashboard 歌曲库后台刷新/手动数据库命令保留为兼容接口，并写入独立的 `library-dashboard.sqlite3` 暂存库；当前 Dashboard 不调用它们。Dashboard 状态只读取 `w4dj.sqlite3`；`save_track_analyses` 只更新 `track-analysis.json` 兼容镜像，只有目标文件元数据事务提交并通过回读校验后，`apply_track_analysis_results` 才按输出 `destination_path` 更新 W4DJ 投影。重新打开只读取缓存，不扫描网易云数据库。
- 增强模式主线程卡顿修复已接入：主线程只做现有 Web Audio 解码/重采样和 MusiCNN 输入准备，Worker 承担同步 Essentia/TensorFlow 计算；取消立即终止 Worker，当前歌曲不写入结果，已完成缓存保留。Dashboard 可对当前输出重新分析，Reload 后从本地暂存恢复继续入口；普通转换覆盖输出会使旧分析失效，高级模型失败仍保留基础分析。目标文件写入后必须重新读取并校验关键字段，校验失败不会标记 W4DJ 分析完成。增强转换和 Dashboard 重分析现在按歌曲立即写回输出元数据、W4DJ SQLite、兼容缓存和历史报告，批次中断不会丢弃已完成歌曲，运行会话报告按歌曲累积；真实长音频上的键盘延迟和数值对照仍需人工验收。
- Danceability 展示已改为固定十级 S 曲线（`app/src/danceability-rating.ts`）：只转换可见等级，保留 Essentia 原始值、查询/排序字段、SQLite/JSON 镜像和 Energy 展示；Joe Fight 约 1.1535 显示 6/10，缺失或非有限值显示 `—`。固定锚点、单调性和边界已有前端回归测试。
- Energy 展示已接入固定十级校准标度（`app/src/energy-rating.ts`）：表格和详情显示星标与 `N/10`，tooltip 保留 `Essentia RMS² raw`；九个边界等于时进入较高一级，缺失/非有限值显示 `—`。原始 Energy 继续用于 SQLite/JSON、筛选、排序和音频标签，Energy 筛选项明确显示为“能量原始值”，不与 Danceability 标度混用。
- 元数据和歌词的代码路径及合成测试已存在；尚未在真实 MP3/FLAC/WAV/AIFF 上逐项复读，也尚未在外部 Rekordbox 中确认所有字段可见。
- 四模型情绪验收工具已完成自动化流程验证，并已从数据库副本生成 5/20/100 请求的 5/10/10 首 manifest；当前 10 条输出仍需要重新分析，旧分析 JSON 不会自动补写新增情绪或 Discogs head。静态 HTTP 服务已用 `python3 -m http.server 1431` 和 curl 完成本地冒烟验证；浏览器播放、主观盲听和模型优劣结论仍待人工执行。100/200 首不能用当前 10 条输出重复伪造，需补充真实输出后再执行。

## 2026-08-22 情绪模型后台接入状态

`HighLevelAnalysis` 现在兼容 `style`、Discogs `genre`、`discogsEffnet` 五个独立 head、`emotionCandidates.emomusic/muse`、`moodCluster` 及各自状态；五个既有 Mood head 仍独立保存。Discogs head 在 Worker 中共享一次 1280 维 embedding、逐个执行并逐个释放 Tensor，缺失/失败/取消只影响自身，不清空基础分析或其它 head。Rust 导入器、Tauri 模型状态和 manifest 映射已加入输入宽度/输入形状、输出层、输出维度、权重长度和暂存重读校验；emoMusic、MuSe、MIREX、Discogs Genre 及五个 Discogs head 资源均已离线安装，没有启用网络下载。旧分析记录仍需用户主动重新分析，不能把模型安装追溯成已完成预测。

## 2026-08-23 Discogs-EffNet 五 head 接入

Discogs-EffNet 现在由一个 `discogs_effnet_embedding` 共享 embedding 和五个独立 head 组成：Mood/Theme（56 类多标签）、Approachability（2 类）、Instrumentation（40 类多标签）、Timbre（2 类）和 Discogs Danceability（2 类）。五个 head 的状态、版本、标签、原始分数、阈值/选中类别、帧数和失败原因写入 `highLevel.discogsEffnet`，并投影到 W4DJ SQLite；缺失或失败的 head 不会清空既有 Style、基础 Danceability、Genre 或其他成功 head。

Dashboard 详情和筛选已支持五个结构化字段，表格可选列默认隐藏。音频写回使用 `W4DJ-Discogs-*` 命名空间字段，保留现有 `W4DJ-Danceability`、标准 Genre、BPM 和 Key。手动错误报告新增 `[Discogs-EffNet 逐 head 状态]`，不会由分析过程自动创建或覆盖报告。

自动化已覆盖资源合同、EffNet Mel 形状/补零、共享 embedding、head 聚合与隔离、SQLite 迁移与保留、查询过滤、Dashboard 展示和手动报告。当前环境尚未对用户提供的真实输出完成新一轮五 head 分析、ExifTool 写回复读、MP3/FLAC/WAV/AIFF 全格式人工验收、Windows、Rekordbox 或浏览器盲听；这些限制不影响离线资源和代码测试结论。

- 新的执行记录见 `docs/superpowers/plans/2026-08-22-emotion-model-backends-and-acceptance.md` 与 `docs/superpowers/plans/2026-08-23-discogs-effnet-heads.md`；三套新增情绪 head 的资源阻塞已解除，Discogs Genre 与五个额外 head 也已接入，但人工盲测和公开字段决策保持未完成，Dashboard 不新增情绪或 Discogs 默认列。真实歌曲重分析和 ExifTool 回读仍待执行。
- Apple Silicon App 已本地构建；DMG 中 Gatekeeper 修复脚本由发布 workflow 注入，当前本地验证没有重新挂载最终 DMG。

## 2026-08-23 断点恢复与性能计划执行状态

本计划 `docs/superpowers/plans/2026-08-23-analysis-resume-and-performance.md` 的代码阶段已完成：运行会话目录写入历史并支持旧记录回退，`analysis-state.json` 采用原子替换，逐曲状态/阶段/帧数/Worker/后端内存指标和 15 秒心跳已持久化；重启后可懒加载会话监视器，单例租约可拒绝并发并接管过期运行。App reload 不自动启动分析，而是提供“继续未完成分析”，已完成歌曲复用数据库结果，未完成歌曲按一首一个 Worker 继续；取消、超时和失败不会写入当前歌曲成功结果。

MusiCNN 已使用预分配连续 Mel 缓冲、64 patch 固定批次、一次双输出 `model.execute()` 和批次级 Tensor 释放；Discogs-EffNet 使用 `[64,128,96]` 流式批次和逐批 head 聚合，不保存整首歌曲的 Discogs `number[][]`。手动错误报告现在读取持久化状态，列出转换与增强分析的总览、所有候选逐曲状态（含 `interrupted`）、最后心跳/阶段快照、模型阶段指标和运行日志；运行会话与错误报告仍只有用户点击导出时才生成文件。

本次验证：前端直接 Vitest 8 个文件 156/156；Tauri 单元测试 49/49；根 workspace `cargo test --all` 429 项；Vite 生产构建、`cargo fmt --all -- --check`、`cargo check --manifest-path src-tauri/Cargo.toml`、Tauri `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 和 `git diff --check` 均通过。最新 Apple Silicon App 构建产物见交接文档。尚未执行真实长音频三次暖运行、目标 macOS WebView CPU/WebGL/WASM 后端对比、9 首 reload/关闭/取消人工验收、Energy 阈值两侧 hover/原始筛选排序人工核对、ExifTool 前后标签对照，以及 Windows/Rekordbox/最终 DMG 验收；这些不被自动化结果替代。

## 2026-08-23 Energy 十级 Dashboard 展示

Energy Dashboard 现在使用校准计划固化的九个 RMS² 分界点映射到 1–10 级星标；表格和详情显示星标与 `N/10`，悬停提示保留 `Essentia RMS² raw` 原始值。`LibraryTrack.energy`、W4DJ SQLite/`track-analysis.json`、原始数值筛选、排序和音频标签均未改变；Danceability 继续使用独立的 S 曲线标度。

本次新增 4 项 Energy 边界/缺失值/格式测试，并扩展 Dashboard 回归。前端直接 Vitest 当前为 8 个文件、156/156（`app` 106、`analysis` 18、`analysis-worker-client` 7、`library-dashboard` 12、`danceability-rating` 4、`emotion-models` 3、`discogs-effnet` 2、`energy-rating` 4）；真实曲库 GUI hover、阈值两侧、原始筛选/排序和外部标签人工核对尚未执行。

## 正在开发/尚未完成的计划项

- FLAC 封面数据库恢复计划已完成代码实施：`NeteaseMetadataResolver` 在批次开始只读加载并通过 `ConversionMetadataContext` 传入普通转换、仅更新元数据、重试和恢复路径；Dashboard 仍只读 `w4dj.sqlite3`，网易云数据库不参与歌曲枚举。匹配诊断区分精确路径、路径后缀、文件名+大小、文件名身份、无匹配和歧义；封面按嵌入、SQLite blob、本地引用、本地缓存、remoteOnly、missing/invalid 的确定顺序恢复，不访问远程 URL。运行会话和手动错误报告记录有效数据库路径、记录数、匹配方法、封面来源、字节数和终止原因。89 首真实 FLAC、ExifTool/GUI 和 WAL/SHM 前后核对仍受当前环境数据与人工操作限制。
- `计划.md` Task 6–10 的代码和自动化路径已完成：包括独立输出歌曲库、A/B 输出根目录状态、失效扫描/清理、列宽/Shift 多列排序/动态操作符、查询竞态防护，以及 Worker 化增强分析工作流。启动和教程后不再自动发现网易云数据库；网易云数据库仅保留在转换元数据链路。
- Task 11 已增加 NcmCore/Enriched 计划、四格式字段映射和写后校验；Task 12 已覆盖索引清理、损坏恢复和本机路径警示。真实格式、真实网易云数据库、Windows 逻辑和 Rekordbox 实际导入仍待环境验收。
- 原始 JSON 可能包含本机路径，当前界面明确警示；基于敏感路径外传限制，未启用把完整原始记录复制到系统剪贴板的动作，详情仍可折叠查看。
- `W4DJ × DJ Crate Digger` 已加入独立未来功能清单，需求来源为 `docs/W4DJ-dj-crate-digger-handoff.md`；目前处于“待确认、未启动”，不属于 Dashboard Task 1–13，也不改变当前优先级。
- 当前没有“正在运行”的后台刷新或失效扫描任务；兼容后台刷新实现已与 `w4dj.sqlite3` 解耦，但 Dashboard 不会启动网易云刷新。真实源目录读到 9 个音频、现有输出目录读到 9 个最终 MP3 和 1 个 `.w4dj-*.wav` 临时文件；已修复目录登记/首次历史导入跳过临时、AppleDouble 和 NCM 源文件，现有用户数据库未被改写。

## 已知问题、风险和技术债务

## 2026-08-23 全局扫描与 FFmpeg 并发

已按 `docs/superpowers/specs/2026-08-23-global-scan-ffmpeg-concurrency-design.md` 接入持久化全局并发上限。`AppPreferences.concurrency_limit` 向后兼容，默认值为 2，运行时规范化到 1–10；扫描和转换任务在启动时复制不可变预算，两个任务槽共享同一组 permit，设置变化只影响下一次任务。扫描使用可取消目录枚举、固定 worker、稳定索引和协调器串行合并；转换使用固定 worker、受限队列、受管理 FFmpeg 子进程和安全提交事务。取消会停止派发并终止本批次 FFmpeg，已提交输出保留，取消错误不计入歌曲损坏；扫描 worker panic 会转为可报告问题；每个槽位显示其任务启动时的并发快照。

自动化已覆盖并发预算规范化、共享上限、可取消等待、偏好迁移、前端控件、稳定扫描结果、扫描 worker 异常报告和 FFmpeg 子进程终止。根 workspace 429 项、Tauri 49 项、前端 156 项、Vite、fmt、check、Tauri 严格 all-targets Clippy 和 diff-check 均通过。仍需真实 M1 音频批次比较并发 1/2 的吞吐、双槽实际重叠、取消 5 秒内退出和 GUI 响应；Windows、Rekordbox 与最终 DMG 仍受环境限制。

- 严格执行 `cargo clippy --all-targets --all-features -- -D warnings` 仍会被既有 legacy binary/module 的 `dead_code` 警告阻断；当前只在明确允许 `-A dead_code` 时通过。不要把这个结果写成“全量严格 Clippy 通过”。
- `refresh_library_catalog` 已改为后台 Tauri command，并通过 `library-refresh-progress` 报告阶段和取消状态；该命令仅作为兼容接口保留，Dashboard 不调用。真实转换数据、跨平台文件选择和长任务取消时序仍未形成验收证据。
- Dashboard 的列宽、Shift 多列排序优先级、动态筛选操作符和查询响应竞态已有自动化覆盖。
- 数据库中只有封面路径/封面存在标记时，Dashboard 仍依赖本地可读音频或可读封面文件；数据库内封面 blob 尚未完整物化为 Dashboard 图片。
- 真实用户网易云数据库、真实普通 MP3/FLAC 音频推理、Rekordbox 导入和 Windows 尚未形成验收证据。内置模型已用真实 TensorFlow.js 逐个完成离线加载和零输入推理，但没有真实歌曲就不能把音频分类准确性列为已验收。
- GUI 拖放视觉和系统文件选择仍需人工验收；模型拖入会被拒绝，内置模型运行不依赖官方模型域名。
- 工作树中存在 `.DS_Store`、`.pnpm-store/`、本地 `W4DJ RKB.app/`、图标副本、脚本、测试夹具/报告和用户创建的“macOS scripts/docs”等 untracked 内容；不要擅自删除或加入提交。

## 2026-08-24 七项计划复核与最新构建

本轮没有重做已完成的代码阶段，只复核真实状态并补跑验证。前端 Vitest 为 8 个测试文件、164/164；根 workspace `cargo test --all` 为 490 项；Tauri `cargo test --manifest-path src-tauri/Cargo.toml` 为 51 项；Vite、Rust fmt、Tauri check、Tauri all-targets Clippy 和 `git diff --check` 均通过。`pnpm` 包装命令仍会在当前环境触发安装/权限策略，Vite/Vitest 使用仓库现有依赖和同一 Node runtime 直接运行。

最新 Apple Silicon 构建位于 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，Info.plist 短版本和构建版本均为 `3.2.0-beta.3`。本轮真实 GUI 复核启动了恢复入口并看到 9 首队列从 0/9 推进到 1/9；旧运行实例的首曲逐曲写回曾返回 `invalid args analyses for command apply_track_analysis_results`，因此该次结果不能作为高级元数据成功证据。当前源码已重新构建，最新构建仍需在用户侧重新点击恢复入口后，用 W4DJ SQLite、`track-analysis.json` 和 ExifTool 逐曲核对。

七项计划仍未全部完成：真实 89 首 FLAC/数据库、完整 Discogs/情绪/Genre 写回、Energy GUI 人工核对、长音频 Reload/取消/恢复、Windows、Rekordbox、DMG 和浏览器盲听仍是未验收项。

## 关键数据结构和接口

- `LibraryCatalog` / `CatalogTrack` / `CatalogLocalFile` / `CatalogSourceRecord`：`src/library_catalog.rs`。
- `LibraryQuery`、白名单字段/操作符、分页和排序：`src/library_query.rs`。
- 网易云发现与 snapshot：`src/netease_library.rs`；底层元数据/封面/歌词候选恢复：`src/netease.rs`。
- 独立输出歌曲库：`src/w4dj_library.rs`；集成测试：`tests/w4dj_library.rs`。
- 独立扫描缓存：`src/scan_cache.rs`；Essentia 分析缓存和 Drop 逻辑：`src/analysis.rs`、`app/src/analysis.ts`。
- 歌词归一化和 `.lrc` sidecar：`src/lyrics.rs`。
- Tauri 入口/命令：`src-tauri/src/main.rs`，包括 `load_library_status`、`locate_netease_library`、`refresh_library_catalog`、`cancel_library_refresh`、数据库兜底选择/清除、`query_library_catalog`、详情/封面/来源记录和缓存清理命令。
- Essentia 模型安装/安全导入：`src-tauri/src/essentia_model_import.rs`；资源位于 `src-tauri/resources/essentia-models`，Tauri 命令为 `restore_bundled_essentia_models` 与 `import_essentia_models`。
- Dashboard UI 和测试：`app/src/library-dashboard.ts`、`app/src/library-dashboard.test.ts`；主界面及状态测试：`app/src/app.ts`、`app/src/app.test.ts`。

## 最近一次完整验证

以下结果已在本次 Energy 接入后重新运行：

- `cargo test --all`：通过，本次运行的根 workspace/unit/integration targets 共 429 个测试用例；新增 Discogs 五 head 投影/保留回归、独立输出库集成覆盖 A/B 根目录、失效清理和未分析记录移除，并覆盖分析回写回读校验、普通转换失效旧分析、临时输出过滤和并发预算/FFmpeg 取消。
- 前端直接 Vitest：8 个测试文件、156 项通过（`app` 106、`analysis` 18、`analysis-worker-client` 7、`library-dashboard` 12、`danceability-rating` 4、`emotion-models` 3、`discogs-effnet` 2、`energy-rating` 4）。覆盖逐歌曲 Worker 创建/销毁、模型启动超时、超时继续、取消、可转移 PCM、情绪/Discogs head 隔离、Energy 十级边界/格式、运行会话导出、并发控件和分析进度局部更新；模型维护按钮和拖入导入入口已移除。
- Tauri 单元测试：通过，49 项测试（含内置模型完整性/恢复、Discogs embedding 与五个 head 的结构/权重/输入输出校验、ZIP 精确配对与冲突拒绝、模型导入安全性、DTO/白名单、运行会话摘要、重启后会话监视器和增量报告回归）。另有 `tests/w4dj_library.rs` 7 项通过。
- 前端生产构建：通过；Vite 提示 bundle 较大（主包约 1.9 MB，Essentia wasm 约 2.5 MB）。
- 情绪验收工具专项：`evaluator.test.js` 与 `main.test.js` 共 8/8 通过；`node --check`、`cargo test --test w4dj_library`（7/7）、`cargo check --manifest-path src-tauri/Cargo.toml`（含 CLI）和 `git diff --check` 通过。`python3 -m http.server 1431 --directory tools/emotion-evaluation` 已启动并通过 curl 页面冒烟；浏览器播放与主观人工验收仍未执行。
- 本次前端直接 Vitest：8 个测试文件、154 项通过，包含手动错误报告/运行会话导出、取消/失败反馈、独立输出库统计/失效扫描控件、Worker 消息路由/旧任务过滤/立即取消/模型启动超时/单曲超时、可转移音频缓冲、情绪与 Discogs head 独立失败、Energy 十级阈值/缺失值/格式、任务 1 自动定位网易云目录与失败后手动兜底、发现进度局部 DOM 更新、hydration 竞态与重复点击锁定、双槽分析进度路由、重新分析当前输出、Reload 恢复入口、逐歌曲持久化、模型维护入口移除、移除网易云 Genre/版权/发布日期表格列，以及 Danceability 十级曲线锚点/边界。前端直接 Vite 生产构建通过，产出独立 `analysis.worker`，仅有已有的大 chunk 提示；`pnpm` 包装命令因当前环境的 `ignored build scripts` 安全策略失败，未作为最终证据。
- 本次 Tauri 测试：49 项通过，包含内置资源完整性/恢复、Discogs embedding 与五个 head 的输入输出与权重长度校验、精确 ZIP shard 配对、多个 `model.json` 冲突拒绝、暂存重读、导入 DTO、URL 白名单、回滚覆盖、运行会话摘要（转换完成与分析超时/待处理分离）、重启后会话监视器和增量运行会话报告。Tauri `cargo clippy --all-targets -- -D warnings` 通过；根工作区严格 all-targets 仍仅被既有 legacy `dead_code` 阻断。
- `cargo fmt --all -- --check`：通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- `cargo clippy --lib --all-features -- -D warnings`：通过。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -A dead_code -D warnings`：通过。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：通过；根工作区严格 all-targets 仍保留既有 `dead_code` 基线失败。
- 两个带 `-A dead_code -D warnings` 的 Rust/Tauri Clippy 检查：通过；严格 all-targets 版本因既有 dead-code 警告未通过。
- Tauri Apple Silicon App：本次全局并发修复后重新构建 `3.2.0-beta.3`，产物为 `/private/tmp/w4dj-tauri-build/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`；Info.plist 的短版本和构建版本均为 `3.2.0-beta.3`，arm64，内置模型资源已包含 MusiCNN、三套情绪 head、Discogs EffNet/Genre 和五个分类 head，前端产物包含独立 `analysis.worker`。这只是本地构建，不等于 Developer ID 发布签名。
- 严格 `cargo clippy --all-targets --all-features -- -D warnings`：未通过，仅报告既有 `dead_code`；这不是交接文档或新功能改动造成的失败。
- `pnpm --dir app test -- --run` 与 `pnpm --dir app build` 包装命令在当前环境因 pnpm 的 `ignored build scripts` 安全策略失败；使用同一安装的 Node/Vitest 与 Vite 直接运行等价命令通过。
- Windows target 检查尝试过，但当前环境缺少离线的 Windows 依赖包且网络代理不可达；未宣称 Windows 完成。
- 真实外部验收记录：当前 Netease SQLite 以 `mode=ro` 打开，`web_track` 可读到 `28712318/FRAGILE`，未找到 `3409113568/SHE DID IT AGAIN`；对现有 9 个 MP3 用 ExifTool/ffprobe 读回标题、歌手、专辑、时长、大小和码率均成功，但尚未完成新一轮 GUI 转换、歌词/封面全链路、Rekordbox 或真实 Worker 长音频操作。
- 尚未完成：真实网易云数据、真实歌曲分类准确性、Rekordbox、Windows 和最终 DMG 挂载验证；模型本身已完成离线加载和零输入推理。

## 下一步优先级

1. 新会话先阅读 `AGENTS.md`、本文件、`docs/handoff.md` 和 `计划.md`，再运行 `git status`，确认不要覆盖现有脏工作树。
2. 完成当前增强分析真实歌曲验收、取消/恢复、SQLite/ExifTool/Dashboard 交叉核对和手动诊断报告闭环。
3. FLAC 封面数据库恢复的代码阶段已完成；下一步只需在可读的 89 首真实 FLAC/网易云数据库上执行只读验收、ExifTool/GUI 核对和最新版 App 构建。
4. 用真实素材复测 MP3/FLAC/WAV/AIFF 元数据写后读取，并在 Rekordbox 实际导入检查 BPM、Key、Genre、Comments 和 Drop LUFS。
5. 在 Windows 或虚拟机验证选择来源、拖放、路径和安装包行为。
6. 若要正式发布，先解决严格 Clippy 的 dead-code 基线和真实验收缺口，再由用户明确授权 commit/push/release。

## 2026-08-24 回写类型兼容修复

旧运行实例在首曲分析写回时暴露了 `invalid args analyses ... expected f64`。源码核对确认原因是高级分析诊断中的 JavaScript `NaN` 经 JSON 序列化变成 `null`，Rust 端的 `FilteredAnalysisLabel.confidence: f64` 无法反序列化。现在该字段使用 `Option<f64>`；前端不可用置信度使用 `null`，Discogs 原始 score map 省略非有限值，并新增前后端回归测试。前端全量 166/166、根 workspace `cargo test --all` 490 项、Tauri check、Tauri all-targets Clippy、Vite 和 fmt 均通过；根 workspace 严格 all-targets Clippy 仍只被既有 `paths_refer_to_same_file` dead-code 阻断。

最新 App 已重新构建到 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`）。真实输出库尚未用该构建重新跑完，当前 9 首仍为 6 首 completed、3 首 failed；因此高级 Genre/Style/Discogs/Emotion 和 ExifTool 回读仍不能标记完成。GUI 访问权限阻止了本轮自动点击恢复按钮，需在用户侧手动点击后继续核对。

本轮曾尝试用 FFmpeg 将一首真实 MP3 送入 Node 下的完整模型链；CPU backend 初始化后进程被当前环境终止，未产生分析结果，也未修改数据库或音频。该尝试不替代 App WebView/Worker 的真实人工验收。

## 2026-08-23 FLAC 封面数据库恢复执行记录

Task 1–6 已完成代码实施。批次开始由 `NeteaseMetadataResolver::load_with_warning` 选择手动偏好或自动候选，SQLite 只读加载为不可变快照；转换、仅更新元数据、重试和恢复都复用 `ConversionMetadataContext`，不在单曲操作中重新读取数据库。`NeteaseRecoveryDiagnostic` 记录数据库状态、匹配证据、曲目/专辑 ID、封面来源、字节数和终止原因，运行会话事件与手动错误报告共享这些诊断。远程 `picUrl` 不访问网络，只标记 `remoteOnly`；有效已有封面保持不变。

本轮自动化：根 workspace `cargo test --all` 468 项通过；Tauri `cargo test --manifest-path src-tauri/Cargo.toml` 49 项通过；前端 jsdom Vitest 10 个文件、168 项通过；Vite 生产构建通过；`cargo fmt --all -- --check`、`cargo check --manifest-path src-tauri/Cargo.toml`、Tauri all-targets Clippy 和 `git diff --check` 通过。根 workspace 严格 all-targets Clippy 仅剩既有 legacy `dead_code` 基线，不把它写成全量通过。

最新 Apple Silicon App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，arm64，Info.plist 版本 `3.2.0-beta.3`。当前环境未提供 89 首 FLAC 源文件/可写真实批次，因此尚未执行真实 FLAC、ExifTool、WAL/SHM 前后和 GUI 人工验收。

## 2026-08-23 手动网易云元数据数据库入口执行记录

已实施 `docs/superpowers/plans/2026-08-23-manual-netease-metadata-database.md` 的代码阶段：任务 1 旁新增“选择网易云数据库”和“恢复自动定位”操作组。选择会先用 SQLite read-only 严格校验支持的网易云 schema，成功后才持久化 `netease_database_path`；无效路径不会覆盖旧偏好，保存失败也会恢复旧值。状态 DTO 使用 camelCase，显示手动路径文件名、有效数据库来源、加载状态、记录数和回退警告。普通转换、仅更新元数据和增强分析写回仍共享批次级不可变 `NeteaseMetadataResolver`，Dashboard 数据源保持 `w4dj.sqlite3`。

本轮新增/通过：严格 resolver schema 拒绝、手动/自动/不可用状态序列化与回退、分析写回批次 resolver、任务 1 UI 渲染、选择取消/错误保留旧状态、恢复自动定位幂等和无副作用测试。当前真实用户数据库、FLAC/MP3 元数据写回、ExifTool 前后对照和 GUI 验收仍需用户实际文件；不会由状态命令自动触发转换或报告导出。

## 2026-08-23 未完成计划统筹元计划

统一执行入口为 `docs/superpowers/plans/2026-08-23-w4dj-incomplete-plans-master.md`。阶段 0 已完成；阶段 1 已落地身份与安全文件名边界、源候选保留、预览碰撞策略、无主跨格式删除保护和 scan-cache schema 2，并通过聚焦 Rust 自动化。手动网易云数据库代码与自动化复核已完成，真实数据库/音频回读仍待现场验收；FLAC 封面、增强分析、Discogs、Energy、情绪模型和 Task 11/13 均按元计划区分代码完成与真实验收缺口。Genre/Style 计划中的“Discogs Head 未完成”属于过期文案，后续计划已实现该部分。

## 2026-08-23 文件名与冲突安全阶段

新增 `src/filename_policy.rs`，把可靠歌曲身份与文件系统安全输出名分开；安全名处理非法字符、隐藏名、Windows 保留名、长度限制和保守碰撞 key，源标签不因文件名清理而改变。来源扫描改用内部路径判别键保留同名/清理后同名的不同源文件；预览在 Rename、Skip、Overwrite、UpdateMetadata 四种策略下显式处理批次碰撞，转换后的跨格式清理不再仅凭清理后的 stem 删除其他文件。scan cache 已升级到 schema 2，旧 schema 只触发安全重扫，不复用旧 derived name。

本阶段自动化：filename policy 4 项单元测试、`cargo test --test sync_policy` 112 项、`cargo test --test preview` 18 项、`cargo test sync::tests` 39 项、`cargo test --test history` 12 项、`cargo test --lib netease` 30 项、`cargo test --test preferences_roundtrip` 5 项、Tauri 51 项均通过；相关 Rust 文件 rustfmt 检查通过。真实 Mass Destruction、89 首 FLAC、Windows 文件系统和人工 GUI 验收未在当前环境执行。

阶段 2 手动数据库现场核对：可读迁移快照包含受支持的网易云表，但 `track` 行数为 0；没有可匹配的 `sqlite_storage.sqlite3` 和 89 首 FLAC 批次。对现有 `Pinch - Qawwali.flac` 与已有 MP3 输出执行了只读 ExifTool 检查，未写入源文件，也未产生 WAL/SHM 副作用。因此阶段 2 的代码/自动化可标记完成，阶段 3 的真实匹配、封面写回和完整回读保留为环境限制。

## 2026-08-23 元计划本轮验收结果

文件名/冲突安全和手动数据库阶段已完成代码与自动化复核。阶段 4/5 的本机自动化也已完成：前端 jsdom 164/164、根 workspace `cargo test --all`、Tauri 51/51、Vite、Tauri check、Tauri all-targets Clippy、Rust 格式检查和 `git diff --check` 均通过。当前 `w4dj.sqlite3` 有 9 条分析结果（6 completed、3 failed）；现有 JSON 的高级 `genre`、`discogsEffnet`、`style` 仍为空，未把模型资源打包校验误报为真实歌曲推理完成。

本轮最新版 Apple Silicon App：`/private/tmp/w4dj-tauri-build/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`。已核对 arm64、Info.plist `3.2.0-beta.3` 和内置 38 个 Essentia/Discogs/情绪模型文件。真实 89 首 FLAC、可匹配网易云数据库、连续长音频/Reload/取消/恢复、Discogs 五 head 回写、Energy hover/筛选排序、浏览器盲听、Windows、Rekordbox、DMG 挂载和 GUI 仍未执行。

## 2026-08-24 模型 Worker 与旧 Mood 输出节点修复

本轮真实 Chromium Worker 验收先复现了模型启动阶段长期没有 `ready` 的问题，根因为跨 Worker 序列化时把多 MB 权重反复转换成 JavaScript `number[]`。现在 `modelWeightDataBuffer` 在 Tauri JSON、Worker `postMessage` 和 TensorFlow `fromMemory` 之间保持二进制缓冲；Worker 在反序列化前先发 `loadingModels` 进度，并把异常转成错误终态。聚焦 Worker/分析测试 31/31 通过。

同一烟测又暴露出随包的六个旧 Mood/Voice 运行时规格把实际 `model/Softmax` 输出写成了 `model/Sigmoid`。已修正 `src-tauri/src/main.rs`，并增加读取随包 JSON 图节点的回归测试。使用 30 秒真实 MP3 临时 PCM、完整 17 个内置模型的 Chromium Worker 烟测结果为：基础分析完成，Discogs Genre 有结果，Discogs 五个 head 均 `completed`，五个旧 Mood/Voice 均返回结果，emoMusic、MuSe、MIREX 均 `completed`；测试只读临时 PCM，不修改用户音频、`w4dj.sqlite3` 或分析 JSON。

真实长音频 `I Feel Love` 已在同一 Chromium Worker 验收中完成完整 17 模型链：基础分析完成，Discogs Genre 有结果，五个 Discogs head、五个旧 Mood/Voice、emoMusic、MuSe、MIREX 均为成功终态；没有写入用户数据库或音频。最终自动化为前端 Vitest 167/167、根 workspace `cargo test --all` 490 项、Tauri 52 项、Vite、fmt、Tauri check、`-A dead_code -D warnings` Clippy 和 diff-check 通过。严格根 workspace all-targets Clippy 仍只被既有 `paths_refer_to_same_file` dead-code 阻断。

尚未完成的是 9 首实际输出的逐曲回写、W4DJ SQLite/兼容 JSON/ExifTool 交叉核对，以及 89 首 FLAC/真实数据库、Energy GUI、浏览器盲听、Windows、Rekordbox 和最终 DMG；这些不能用单首只读 Worker 验收替代。

最新 Apple Silicon App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`，内置模型资源 38 个文件）。

## 2026-08-24 后台启动与最新状态核对

后续验收统一使用 `open -g` 后台启动 App，不调用普通 `open -a`、`activate` 或 `AXRaise`。最新构建进程保持运行，Info.plist 为 `3.2.0-beta.3`，架构为 arm64，bundle 内含 38 个 Essentia/Discogs/情绪模型资源。

本轮发现 `app/src/analysis.ts` 的 MusiCNN 批次常量存在导出声明和内部同名声明，已删除重复内部声明并重新运行前端测试、Vite 和 App 打包。当前前端 Vitest 为 170/170，根 workspace `cargo test --all`、Tauri 52 项、fmt、Tauri check、Tauri all-targets Clippy、根 Clippy（放宽既有 `dead_code`）和 `git diff --check` 均通过。

计划要求的根 workspace 严格 `cargo clippy --all-targets --all-features -- -D warnings` 也已重跑，仍只被既有 `src/sync.rs:3378 paths_refer_to_same_file` 的 `dead_code` 阻断，未出现本轮新增警告。

只读核对用户当前 `w4dj.sqlite3` 得到 9 条 `/Users/mac2/Music/test` 输出分析记录：7 条 SQLite `completed`、2 条 `failed`。但 9 条中只有 Hallelujah 的 `highLevel.status=completed`；其余旧记录保留了高层失败信息，其中包括 `Can't find variable: MUSICCNN_INFERENCE_BATCH_SIZE`。这些是历史分析结果，不能冒充本次最新 App 的真实高级元数据验收；后台启动也不会自动触发重分析。仍需用户在最新 App 中手动点击继续/重新分析后，再做 SQLite、兼容 JSON 和 ExifTool 交叉核对。

## 2026-08-24：MusiCNN 长音频内存边界修复

后台 `open -g` 复现旧构建时，WebContent 在 `Hallelujah` 的 17,346 帧 MusiCNN Mel 提取中达到约 1.6–2.4 GB，并发生 WebView 重建；截图中的 `extractingMusiCnn` 进度证明任务确实在运行，失败点是 Essentia 原生逐帧分配的生命周期边界，不是模型 IPC 或 SQLite 回写。现在生产 runtime 以 256 帧为块创建/销毁 Essentia 实例，块内仍显式释放 frame、bands、FrameGenerator，保持 512/256、187×96、补零和进度协议不变；新增分块释放和行顺序测试。

本轮最新自动化：前端 Vitest 173/173、根 workspace `cargo test --all` 490 项、Tauri 52 项、TypeScript、Vite、Rust fmt、Tauri check、Tauri Clippy、根 `-A dead_code -D warnings` Clippy 和 diff-check 通过。最终正常入口 Apple Silicon App 为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（`3.2.0-beta.3`）。严格根 all-targets Clippy 仍只受既有 `src/sync.rs:3378 paths_refer_to_same_file` dead_code 阻断。

当前真实数据尚未在最新版 App 中完成整库回写；本轮未用自动脚本触发用户数据库写入，因此 SQLite/兼容 JSON/ExifTool 的全批次结果、取消/恢复和 89 首 FLAC 仍不能标记为通过。现场只读状态为 `tracks=16`、`analysis_results=8 completed/1 failed`，其余歌曲未分析。
## 2026-08-24：授权整库重分析现场停止

按用户授权使用 `open -g` 启动临时验收包并触发歌曲库重分析。只读核对确认本次按钮候选受当前任务槽输出根目录限制，实际进入的是 `/Users/mac2/Music/test` 的 9 首“当前输出”，不是独立库中的全部 16 首；运行期间没有创建运行会话目录。随后检查发现 `w4dj-desktop` 已退出，5 秒内 `w4dj.sqlite3` 与 `track-analysis.json` 的 mtime 均保持不变，SQLite 状态仍为 `completed=8 / failed=1`、`tracks=16`，因此停止等待并保留所有已有结果，未删除或重置数据库/JSON。

为避免 WebKit `decodeAudioData` 对异常/长 MP3 无限 pending，本轮保留 `app/src/analysis.ts` 的 300 秒解码超时保护；临时触发器和诊断标记已清理。正常 arm64 App 已重新构建，真实整库高级回写仍未通过验收。

## 2026-08-24：整库重分析候选范围修复

`list_library_analysis_candidates` 已解除对任务槽当前 `destination_directory` 的候选过滤。Tauri 现在直接使用 `w4dj.sqlite3` 中 `available` 且可读的本地音频；配置中的输出根目录只用于尽可能保留 `slotIndex`，不再决定歌曲是否进入分析队列。这样独立库中位于其他输出根目录的歌曲也会进入重分析。新增回归测试覆盖配置根目录外的 FLAC 和非音频文件过滤。尚未用该修复后的最新版 App 执行新的 16 首真实回写验收。

本轮验证：前端 Vitest 全量 173/173、Tauri 候选回归测试通过；Vite、Tauri `cargo check`、arm64 App 构建和 `git diff --check` 通过。App 版本仍为 `3.2.0-beta.3`。

## 2026-08-24：验收入口改为无 GUI 后台模式

此前 `open -g` 启动后依赖 WebView 可访问性按钮的方案不再用于后续验收：后台窗口可能只暴露标题栏或空可访问性树，不能作为稳定触发接口。新的统一规范为 `docs/superpowers/plans/2026-08-24-headless-acceptance.md`。后续先实现隐藏 WebView 验收运行时与共享分析 runner，再通过命令行场景直接启动 16 首和后续批次；W4DJ GUI 不显示、不聚焦，也不需要按钮。

数据验收继续以进程、JSONL、`w4dj.sqlite3`、`track-analysis.json`、文件 mtime、ffprobe 和 ExifTool 为证据。Energy 使用 jsdom/隐藏 WebView 验证 DOM 与排序筛选；主观盲听和 Rekordbox 实机导入属于外部人工项目，不能由后台程序伪造通过，也不要求打开 W4DJ App GUI。当前仅修改验收规范，隐藏运行时尚未实施，16 首真实整库结果仍保持未完成。

## 2026-08-24：无 GUI 后台验收执行结果

隐藏 `headless.html`、`--headless-acceptance` 参数、JSONL 事件和共享分析 runner 已实施，最新 App 为 arm64、`3.2.0-beta.3`。三次直接运行 `libraryAnalysis --exercise-cancel-resume` 均在首曲 MusiCNN 提取接近 17,344 帧时 WebContent 重启，报告 `/private/tmp/w4dj-headless-acceptance/library-analysis-16-final.jsonl` 出现 3 个 `runId`，没有 `persisting` 事件。App 已正常终止，SQLite/JSON 保持 `completed=8 / failed=1 / notAnalyzed=7`，没有写入用户音频；16 首整库、取消/恢复和 ExifTool 交叉验收保持未完成。下一步先修复隐藏 WebContent 的 Essentia/TensorFlow 内存或进程重启，再重跑。

本轮自动化：前端直接 Vitest 178/178、TypeScript、Vite、Rust fmt、Tauri `cargo check` 和 `git diff --check` 通过；根 `cargo test --all` 当前有 3 个 filename fallback 断言失败，未将 Rust 全量写成通过。最新产物为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app` 及同目录 arm64 DMG。

## 2026-08-24 增强分析入口临时隐藏

为便于当前阶段的稳定性调试，前端使用 `app/src/app.ts` 中的单一开关
`ENHANCED_ANALYSIS_FEATURES_VISIBLE = false` 同时隐藏增强模式选择器、Essentia
预训练模型状态区以及增强/扫描缓存清理入口。隐藏只影响普通转换界面，不删除
Worker、模型加载、分析回写或 Tauri 命令；后端能力继续保留，现有缓存和分析结果也
不会被清理。`src-tauri/src/main.rs` 在每次启动加载偏好后强制设置
`enhanced_mode = false`，因此每次新启动默认不开启增强模式，即使旧偏好曾经保存为开启。

后续调试完成后，恢复这三个入口只需将该前端开关改为 `true` 并重新构建 App；后端
命令无需恢复或迁移。恢复前仍应保留启动默认关闭策略，用户可在重新显示的入口中按
需开启当前会话。

## 2026-08-25 歌单导入、二维码与 M3U8

已实施计划 `docs/superpowers/plans/2026-08-24-w4dj-playlist-qr-import.md` 的代码阶段；当前活动协议已切换为全新的最小 `.w4dj` v2。v1 文件和旧字段明确拒绝，不做迁移。`w4dj.sqlite3` 歌单持久化、Tauri 导入/查询/TXT/W4DJ v2 导出、前端本地 QR 分页与整窗拖入、独立 W4DJ 输出匹配、手动覆盖和相对路径 M3U8 原子导出仍保留。匹配只读取 `available` 且可读的 W4DJ 输出，不读取网易云 SQL、旧 Dashboard 投影、转换历史或分析 JSON；导出时通过稳定 `track_key` 重新解析当前路径。

本轮聚焦验证：`dj_playlist_match` 5/5、`w4dj_library` 7/7、`m3u8` 4/4、Tauri 单元测试 57/57、前端 `dj-playlist` 4/4、`qr-code` 3/3、`app.test.ts` 112/112、TypeScript、Vite 构建和 `cargo check` 均通过。现有工作树仍包含大量既有修改，未提交、未推送、未改版本。真实手机扫码、网易云粘贴、10 首输出匹配、M3U8 播放器/Rekordbox 导入尚未执行。

收尾复核：前端全量 Vitest 为 26 个文件/380 项，歌单匹配、独立库和 M3U8 Rust 针对性测试仍为 5/5、7/7、4/4，Tauri 测试 57/57，TypeScript、Vite、`cargo check` 均通过。arm64 App 已重新构建于 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，Info.plist 版本为 `3.2.0-beta.3`。根 workspace 全量测试仍保留 3 个既有 filename fallback 断言失败；严格 Tauri Clippy 仍被既有 unused-import/collapsible-if 警告阻断。DMG bundler 在当前环境失败，原因是生成的 `bundle_dmg.sh` 未收到有效参数，手动 `hdiutil` 也返回“设备未配置”，因此没有新的可挂载 DMG。

随后已清理 Tauri 歌单 TXT 导出路径的未使用导入与 `collapsible_if`，严格 Tauri Clippy（`--all-targets -- -D warnings`）及 57 项 Tauri 测试再次通过。全仓 `cargo fmt --all -- --check` 仍受工作树既有的跨文件格式差异影响，未对无关改动执行批量格式化。

歌单计划完成度补充：应用启动时读取 `w4dj.sqlite3` 的最近导入歌单摘要，用户可通过“打开最近歌单”加载完整持久化记录和已有匹配；Tauri 原生 `.w4dj` 拖拽在 `enter/over` 期间持续显示全窗口模糊覆盖层。生产 Rust 解析器对 Afro House 样本的隔离验收为 10 首、0 警告、386 UTF-8 字节（无尾部换行），控制输出 fixture 达到 10/10 匹配并生成 10 条相对路径 M3U8，所有路径均回读为可读文件。新增 UI 后的前端全量为 26 文件/386 测试通过。手机扫码、网易云粘贴、外部播放器和 Rekordbox 仍是未执行的人工验收；DMG 仍受当前环境设备限制。

最终复跑 `cargo test --all` 退出码为 0，根 workspace 全量测试通过；之前记录的 3 个 filename fallback 失败本次未重现。全仓 fmt check 的既有跨文件差异仍保留并已如实记录。
## 2026-08-25 DJ playlist QR/M3U8 acceptance follow-up

The supplied `afro-house-club.w4dj` sample is accepted by the production parser and isolated schema-v2 library persistence (10 ordered tracks, 386 UTF-8 bytes, no warnings). A controlled 10-output fixture reaches 10/10 deterministic matching and produces an atomically written relative extended-M3U8 whose 10 paths all resolve to readable files. The persisted recent-playlist action and native `.w4dj` drag-over overlay are covered by the current frontend suite.

Phone QR scanning/NetEase paste, external-player playback, and Rekordbox import remain unexecuted environment checks. The exact manual sequence is: scan every QR page with a phone and compare the decoded text; paste the copied all-pages text into NetEase; open the exported M3U8 in IINA/VLC or another compatible player and verify all entries/order; import the same file in Rekordbox and verify all entries/order. No result is claimed until those steps are performed. A new DMG was not produced because the bundle script failed before invocation and `hdiutil` reported `设备未配置`; the arm64 `.app` remains the available artifact.

Final verification update: the 100-track QR pagination fixture now passes; the current frontend run is 26 files/388 tests. Root Rust tests, Tauri tests, focused playlist/match/M3U8 tests, TypeScript, Vite, fmt, check, strict Clippy, and diff-check all pass. The arm64 `.app` was rebuilt at 2026-08-25 01:00:33 with version `3.2.0-beta.3` and the bundled FFmpeg sidecar. Phone/NetEase/player/Rekordbox steps remain unexecuted; the exact manual sequence above is still required. DMG remains unavailable because of the recorded bundler/device errors.

### 2026-08-25 高级模型懒加载

- [x] 删除普通启动阶段的内置 Essentia 模型校验/复制；启动只初始化模型路径，不读取或处理模型文件。
- [x] 新增 `ensure_essentia_models` Tauri 命令，首次增强分析时在写锁内校验并补齐内置模型，然后再通过现有命令读取模型并交给 Worker。
- [x] Dashboard 与隐藏分析 runner 均改为显式增强分析触发懒初始化；普通转换不再触发模型状态查询、安装或 Worker 模型加载。
- [x] 前端 Vitest 191/191、TypeScript、Vite、Tauri 测试 57/57、Tauri check、严格 Tauri Clippy 和 Rust fmt check 通过。
- [x] 最新 Apple Silicon App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`，arm64，内置模型资源 38 个。

## 2026-08-25 快速数据库扫描与转换后增强分析

已按 `docs/superpowers/plans/2026-08-25-fast-database-scan-and-preconversion-analysis.md` 收口当前实现。网易云数据库发现现在先走 `probe_netease_database` 的只读 schema/行数探测，再通过 `load_records_from_db_observed` 以有界并发读取支持表；结果按数据库 fingerprint（路径、大小、mtime、WAL/SHM mtime）做进程内缓存复用。自动发现会先并发探测候选数据库，只加载记录数最多的支持库；手动路径先校验 schema，无效时立即回退自动发现。任务 1 的 `locate_netease_library(true)` 现在先返回数据库/音乐目录，目录文件数通过后台 `netease-discovery-progress` 的 `checkingMusicFolder`/`completed` 事件补发，不再阻塞路径返回。

扫描与增强分析的阶段边界保持清晰：普通模式严格按“扫描 → 数据库准备 → 转换”执行，增强模式默认关闭，只有显式开启增强模式或歌曲库手动“重新分析当前输出”时，才在转换完成后进入逐曲 Worker 分析与标签写回。任务卡进度条同时承载数据库发现阶段（`locatingDatabase`、`readingRecords`、`checkingMusicFolder`）和转换后增强分析阶段；前端高频进度事件只更新局部 DOM，不重建整棵界面。

本轮新鲜验证：`cargo test --test netease` 4/4、`cargo test --test sync_policy` 115/115、`cargo test --test library_catalog` 18/18、`cargo test --manifest-path src-tauri/Cargo.toml` 57/57、`cargo fmt --all -- --check`、`cargo check --manifest-path src-tauri/Cargo.toml`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、前端 Vitest 12 文件 191/191 与 Vite 生产构建全部通过。当前 Codex 无 TTY 环境下 `pnpm --dir app test -- --run` 仍会因 `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY` / `ERR_PNPM_IGNORED_BUILDS` 中断，因此前端验证继续使用同一 `node_modules` 上的直接 `vitest`/`vite` 调用完成。

真实数据侧，本机只读核对到 `/Users/mac2/Library/Containers/com.netease.163music/Data/Documents/storage/sqlite_storage.sqlite3`：支持表为 `track` 和 `web_track`，当前计数分别为 0 和 538；read-only 探测前后 SQLite `mtime` 不变。计划中要求的“外置 T7 数据库 + 2,398 输入文件”完整时序验收在本轮仍未执行，因此外置卷 I/O、完整扫描耗时与转换起始时间仍保留为待现场验收项。

最终收口：扫描 worker 保持单次文件处理，任务开始额外做可取消的轻量预枚举以确定分母；取消、问题和总数通过 `enumerate_music_files_observed` 回报。前端全量直接 Vitest 26 个文件/390 项通过，TypeScript 与 Vite 生产构建通过；根 workspace `cargo test --all`、网易云/扫描/目录聚焦测试、Tauri 57 项、fmt、Tauri check、Tauri 严格 Clippy 和 `git diff --check` 通过。最新 arm64 App 已重新编译，仍为 `3.2.0-beta.3`。真实 T7/2,398 文件和完整 SQLite/JSON/ExifTool 交叉验收因当前环境未挂载对应数据仍未标记通过。
## 2026-08-25：重复歌曲专辑消歧验收

已实施 `docs/superpowers/plans/2026-08-25-duplicate-track-disambiguation.md`。预检现在先保留网易云 `trackId/albumId`、专辑和原始路径，只对实际映射到同一目标路径的候选组追加最短消歧后缀；非冲突歌曲沿用原文件名。转换元数据仍从原始来源身份写回，后缀不会污染 Title、Artist、Album 或其他已有 W4DJ 标签。最终路径通过 `w4dj.sqlite3` 独立记录，网易云源数据库保持只读。

两首真实 `STONE KOLD` 冲突曲目已验收通过：`STONE KOLD - Skybreak,Subten.ncm`（trackId `2707606350`，专辑 `STONE KOLD`）与 `STONE KOLD - Skybreak,Subten (1).ncm`（trackId `2714172644`，专辑 `HALF BLOOD`）均转换成功并生成不同目标路径；两首输出的 Title/Artist/Album 与源记录一致。已有 `/Volumes/T7_1T/Neteast/test/STONE KOLD - Skybreak, Subten.mp3` 在重跑预览中保留，另一首使用专辑消歧路径，不覆盖旧文件。W4DJ 身份和专辑分别登记。

本轮验证：真实双曲验收测试通过；前端 Vitest 191/191，根 `cargo test --all`，Tauri `cargo check`、严格 Clippy、TypeScript、Vite、Rust fmt 和 `git diff --check` 均通过。当前仍未把外置批次中 100 首非冲突歌曲和全部高级标签逐文件 ExifTool 回读写成已完成验收。

## 2026-08-25：轻量网易云元数据定位缓存与快速启动

已按 `/private/tmp/W4DJ-lazy-netease-metadata-cache-handoff.md` 增量实施。启动阶段现在只做网易云数据库的只读 schema/行数探测和轻量缓存摘要读取，不再把 `track`/`web_track` 的完整 JSON、歌词或封面载入内存。新增 `library-dashboard.sqlite3` 中的 `netease_cache_meta` 与 `netease_track_locators` 表，仅保存 track ID、来源表/rowid、数据库 fingerprint、规范化路径/文件名、文件大小和最小标题/艺人/专辑匹配键；源网易云 SQLite 始终以 read-only 打开。

缓存构建通过 `prepare_netease_metadata_cache`、`cancel_netease_metadata_cache`、`load_netease_metadata_cache_status` 提供后台单例、进度事件、协作取消和 fingerprint 失效检测；事务完成前不会替换旧快照。转换/扫描第一次触达元数据边界时才准备缓存，候选歌曲匹配后按来源 rowid 单曲读取完整元数据，因此完整记录、歌词和封面不会在启动或批量预加载阶段进入内存。任务 1 扫描期间显示轻量缓存阶段，取消扫描也会取消正在构建的缓存；普通文件夹没有网易云数据库时仍可继续扫描/转换。

本轮验证：前端直接 Vitest 26 个文件、392 项通过；TypeScript 检查、Vite 生产构建、根 `cargo test --all`、Tauri `cargo test`（57 项）、根 `cargo check --all-targets`、Tauri check 和 `cargo fmt --all` 通过；新增轻量 locator 缓存 round-trip、无 raw metadata 列和 WAL/SHM fingerprint 失效测试通过。根 workspace 严格 `cargo clippy --all-targets -- -D warnings` 仍受既有二进制 target 的大量 legacy `dead_code` 及一个既有 test `map_identity` 阻断；`cargo clippy --lib -- -D warnings` 和 Tauri 严格 Clippy 通过。Vite 已重新构建，arm64 App 产物为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`。

外置 T7 数据库、2,398 首输入文件的冷启动/扫描时序，以及真实用户 App 中的逐曲元数据写回仍未执行；本轮没有修改网易云数据库、音频、`w4dj.sqlite3` 或 `track-analysis.json`，也未提交或推送。

## 2026-08-25 扫描进度显示修复

扫描事件现在分别维护输入目录与输出目录的计数。扫描阶段总数未知时，任务卡显示“已扫描 N 项”并使用不定长进度条；只有阶段总数确定后才显示 `N/总数`，并拒绝显示旧事件造成的不可能比例。任务卡在扫描期间使用“运行中”状态，避免显示“待命”。旧版没有分目录字段的事件仍通过兼容回退处理。

本轮验证：前端全量直接 Vitest 26 个文件/392 项、TypeScript、Vite、Tauri 57/57、`sync_policy` 117/117、Tauri check、严格 Tauri Clippy 和 `git diff --check` 通过。全仓 fmt check 仍受工作树中既有的 `src/netease.rs`/`src/netease_cache.rs` 格式差异影响，未格式化无关改动。最新 arm64 App 为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`。

## 2026-08-25 任务 1 网易云数据库命名与元数据补全

已按 `/private/tmp/W4DJ-task1-netease-db-filename-metadata-handoff.md` 在现有脏工作树上增量实施。任务 1 的 `PreserveSource` 预览在计算冲突、已有输出和重复消歧之前先读取只读网易云 resolver；匹配成功时使用数据库 Title/Artist 按用户文件名规则生成目标路径，`Original` 仍保留源 basename。候选同时携带数据库 Title/Artist/Album、track ID 和 album ID，后续标签、封面和分析回写不会再次重命名。任务 2 的 `SoundCloud` 路由不使用该数据库身份做路径或标签补全。

无标签 MP3/FLAC 的任务 1 写回复用同一 resolver，仅填充缺失身份字段；可靠源标签不会被覆盖。为当前真实样本补强了全角/弯引号规范化、已知音频扩展名 stem 比较和 `web_track` 无路径记录的惰性 locator 文件名键。自动化已覆盖 resolver-aware 预览、MP3 标签回退、任务 2 隔离、locator round-trip 以及真实 `Mass Destruction` 数据库身份匹配；源 FLAC/MP3、网易云 SQLite、W4DJ SQLite 和既有输出均未修改。

尚未执行需要用户真实素材的重新转换、FLAC/MP3 ExifTool 写后回读、旧输出批量改名或外置 T7 全批次验收。版本保持 `3.2.0-beta.3`，未提交或推送。

本轮真实只读/临时输出验收补充：当前网易云容器数据库与 T7 上的 `Mass Destruction` 无标签 FLAC 可匹配；任务 1 预览生成数据库身份路径，临时 MP3 写回后读到数据库 Title/Artist/Album，源 FLAC 字节未变化。最新 arm64 App 已用 `cargo tauri build --target aarch64-apple-darwin --bundles app --no-sign` 构建，Info.plist 为 `3.2.0-beta.3`；根 `cargo test --all`、Tauri 57/57、前端 192/192、TypeScript、Vite、fmt、Tauri check、Tauri Clippy 和 diff-check 通过。根严格 all-targets Clippy 仍只受既有 `dead_code` 与 `map_identity` 阻断。
## 2026-08-25：W4DJ 最小歌单格式 v2（当前状态）

已按 `docs/superpowers/plans/2026-08-25-minimal-w4dj-playlist-format.md` 完成新格式代码阶段。当前协议严格固定为 v2：只读写 `format`、`format_version`、`export_id`、`playlist.name`、`tracks.position/title/artist_display/netease_track_id`。旧 v1 文件（包括用户提供的 `uk-bass-simulated-10.w4dj`）及 `duration`、`platform_refs`、`dedupe_key`、本地路径等旧字段不会被迁移或兼容读取；错误提示要求重新导出 v2。

网易云 ID 作为字符串保存在导入记录和 `w4dj.sqlite3` 的实际输出身份映射中。匹配先按 ID 查询 `available` 且可读的 W4DJ 输出，再在缺少 ID 时执行唯一标题+歌手回退；相似标题、文件名提示不会覆盖 ID 命中，歧义结果不能自动选择。W4DJ v2 导出由用户选择保存路径后手动生成，M3U8 使用数据库当前 `destination_path`，重复 position 保留重复行。

本轮自动化通过：根 `cargo test --all`、Tauri 57 项测试、前端 Vitest 192 项、TypeScript、Vite、`cargo check --manifest-path src-tauri/Cargo.toml`、严格 Tauri Clippy、`cargo fmt --all -- --check`、`git diff --check`。当前版本仍为 `3.2.0-beta.3`，未提交、未推送。真实手机扫码、网易云粘贴、播放器/Rekordbox 导入和外置卷现场验收未执行。

最新版 arm64 App 已重新构建并核对：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`；Mach-O 为 arm64，Info.plist 的短版本和构建版本均为 `3.2.0-beta.3`。

## 2026-08-25 扫描取消即时响应与动态总数

本轮修复了任务 1 扫描“长时间只显示已扫描 1088 项”和取消按钮延迟的问题。后台扫描在输入预枚举、输入/输出扫描、网易云元数据匹配阶段共用可取消标记；`cancel_scan` 先同步发布 `cancelling`，后台逐条检查后发布 `cancelled`，重复取消幂等。扫描开始前以同一套可取消枚举确定输入/输出分母，因此任务卡可以从 `0/总数` 显示到 `N/总数`；元数据匹配单独维护计数。

网易云扫描使用轻量 locator 的路径/文件名/stem 索引，不在每首歌匹配时打开完整 SQLite，也不在扫描开始调用完整 `load_exact`。前端 120ms 轮询只修改任务卡文本、进度条宽度和不定长 class，取消状态和终态才进行完整渲染。

真实只读验收使用 `/Volumes/T7_1T/Neteast/test`：1088 首候选、1088 个元数据事件，索引预览 3.52 秒；未修改源数据库、音频、`w4dj.sqlite3` 或分析 JSON。自动化通过：前端 Vitest 194/194、TypeScript、Vite、根 `cargo test --all`、Tauri 58/58、Tauri check、Tauri 严格 Clippy、Rust fmt 和 diff-check。根 all-targets 严格 Clippy 仍受工作树既有 legacy `dead_code` 与 `duplicate_track_acceptance` 的 `map_identity` 阻断；真实 GUI 点击取消、Windows/Rekordbox 和外置卷完整现场验收未执行。

最新 arm64 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，Mach-O arm64，版本 `3.2.0-beta.3`。

## 2026-08-27 Discogs EffNet 内置资源去重

已移除 App 内置资源中的重复 `discogs_effnet.{json,bin}` 旧副本，只保留正式的
`discogs_effnet_embedding.{json,bin}`。分析运行时本来就优先加载 canonical embedding，
模型状态检查仍兼容旧 ID；导入器继续接受用户手动导入的旧模型，因此不会破坏既有用户模型。
同时更新离线资源生成脚本、资源说明和校验测试，防止后续重新生成时再次复制旧副本。

内置模型资源从约 47.3 MiB 降至约 29.9 MiB，arm64 App 从约 112.9 MiB 降至约 95.5 MiB。
Tauri 资源校验 64 项（含旧 ID 导入兼容和 canonical-only 资源断言）、根 Rust 全量测试、
格式检查与 diff-check 通过；版本保持 `3.2.0-beta.3`，未修改用户模型目录、音频或数据库。

## 2026-08-28 零启动扫描与轻量输出索引

已按 `/private/tmp/W4DJ-zero-startup-scan-lightweight-index-handoff.md` 在共享工作树增量实施。挂起报告确认的启动阻塞链
`import_initial_history → upsert_output_file → read_track_metadata → recover_local_metadata → load_records_cached → merge_table_records`
已从 Tauri setup 移除；setup 现在只初始化路径、读取偏好并打开私有 `w4dj.sqlite3`，不会导入旧 `history.json`、遍历旧输出、探测媒体、读取网易云歌曲表或加载模型。兼容 `import_initial_history` 方法仍可由显式调用使用，但生产启动不再进入该路径。

普通转换的最终安全提交回调现在调用 `W4djLibrary::upsert_lightweight_output`：登记稳定 ID（网易云 ID→来源→目标路径）、来源/目标、槽位及最小 Title/Artist，不读取文件元数据，不调用 probe/NCM 完整记录，不登记或切换 `output_roots`/`slot_output_roots`。同一 ID/来源迁移到新目标仍是一条记录；提交新文件会清除旧分析投影并重置为 `notAnalyzed`，旧音频不被访问或删除。旧的转换后输出目录扫描已删除。

Dashboard 分析候选、DJ 歌单匹配和 M3U8 候选改为使用轻量索引中的全部记录；不再依赖 `available` 等兼容状态，实际导出仅检查所选路径。网易云 SQLite 与 `library-dashboard.sqlite3` 仍只承担按需元数据/兼容职责，不枚举 W4DJ 歌曲。

本轮验证：根 `cargo test --all` 全部通过（含根库 115、Tauri 101、集成测试），Tauri `cargo check` 与 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 通过；根 Clippy 在放宽既有 `dead_code` 与 `tests/duplicate_track_acceptance.rs` 的 `map_identity` 后通过，严格模式仍保留这两个既有问题；前端直接 Vitest 12 文件/198 项、TypeScript、Vite 生产构建、`cargo fmt --all -- --check` 和 `git diff --check` 通过。兼容直转入口现在传递可获得的网易云身份，输出替换清理限制在当前根目录；当前 App 版本仍为 `3.2.0-beta.3`，未 commit/push。

真实跨用户全新/升级用户后台启动验收已完成：对最新 arm64 App 临时副本通过 `open -g` 启动包装副本，在隔离 `HOME/TMPDIR` 下全新用户 1 秒内观察到实际 arm64 进程并创建空 `w4dj.sqlite3`（0 首、0 分析），升级用户 1 秒内启动；升级副本的五个核心表行内容和目标路径与生产副本一致，没有历史导入或输出遍历。辅助功能权限未授予，窗口数量只能记录为 `access-denied`，不把它伪写成窗口可访问性通过。最新 App 构建时间为 2026-08-28 15:17:01 CST，版本 `3.2.0-beta.3`。

真实转换后索引落库/分析回写、外置 T7/2,398 文件性能、89 首 FLAC、Windows、Rekordbox 和人工 GUI/播放器验收仍需对应环境；后续验收继续使用 `open -g`，不调用 `activate`、`AXRaise`、截图或 `view_image`。

## 2026-08-28 生产包隔离验收资源

此前约 200 MB 的 App 是因为 `app/public/acceptance-audio`（约 94 MB 的验收音频）和
`app/public/essentia-models`（约 31 MB 的模型副本）被 Vite 复制进 `app/dist`，再被 Tauri
嵌入主程序；模型同时还存在于正式的 Tauri Resources 中。现已将生产配置的 `publicDir`
设为 `false`：`app/public` 继续留在工作区供验收工具按需使用，但不会进入生产 `dist` 或
App。正式包仍保留 Tauri Resources 中的一份 Essentia 模型和现有隐藏运行时入口。

重新构建后 `app/dist` 约 6.4 MB，arm64 `.app` 约 95 MB（主程序约 18 MB、ffmpeg 约
43 MB、正式模型约 30 MB）；包内没有验收音频、验收静态页面或前端模型副本。前端
Vitest 198/198、TypeScript、Vite 构建和 arm64 App 体积/路径核对通过，版本保持
`3.2.0-beta.3`，未修改用户数据、未 commit/push/release。

## 2026-08-28 任务 1 网易云“情况”栏

任务 1 网易云来源工具栏现在固定显示“情况”/“Status”标签，动态值集中由前端状态解析器生成，覆盖状态读取、索引未就绪、建立中、已就绪、取消、有效手动数据库、自动回退和错误。错误、警告、进行中和成功使用对应语义色，完整消息保留在 `title` 中，窄屏仍可流式布局。缓存建立期间只原位更新情况值；按钮状态或终态变化时才重绘工具栏，避免任务卡和输入框跳动。网易云扫描、数据库选择与恢复自动定位按钮及后端接口未改变。

本轮前端 app 回归测试 124/124、TypeScript、Vite 构建和 `git diff --check` 通过；最新版 arm64 App 已重新编译，版本保持 `3.2.0-beta.3`。真实网易云数据库、Windows/Rekordbox 和播放器现场验收仍按既有环境限制待执行。

## 2026-08-28 任务 1 扫描进度与转换预览口径修复（独立 worktree）

本轮在 `/private/tmp/w4dj-task1-scan-preview-worktree` 增量修复了任务 1 的进度口径。后端不再把输入、输出和元数据阶段的 processed 相加作为最终分母；完成态使用输入源 `source_processed/source_total`，因此不会再把输出目录的 `73/73` 显示成扫描总结果。网易云轻量缓存准备阶段现在映射到任务 1 的准备进度，扫描完成/取消/错误快照在下一次操作前保留。

`SyncPreview` 新增可选的输入总数、输出重复数、当前策略操作数、实际数据库目录和逐曲明细。确认窗口将卡片改为“输入曲目/输出重复曲目/将跳过或将覆盖或将更新元数据/错误文件”，预计输出和可用空间在右侧同一列分两行显示；四张卡片可按 A–Z 打开懒加载明细，并通过现有安全文件打开命令访问源文件或目标文件。旧 DTO 字段仍保留兼容读取。

独立 worktree 的 app.test.ts 为 126/126，TypeScript、Vite、Tauri check 和根 Rust 基线测试通过。真实 1173 首扫描、覆盖模式转换、数据库目录实际值和文件打开链接仍待用户环境验收；主工作树未被修改。

独立 worktree 最终验收：前端 Vitest 14 文件/209 项、TypeScript、Vite、根 Rust 全量测试、Tauri check、严格 Tauri Clippy、fmt、diff-check 均通过。最新 arm64 App 位于 `/private/tmp/w4dj-task1-scan-preview-worktree/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`，包体约 95 MB。真实 1173 首目录现场扫描和 GUI 文件打开仍待用户环境。

## 2026-08-29 扫描本地网易云文件夹后台化

任务 1 的网易云目录发现改为后台单例 worker：命中标准目录时命令立即返回并先填入任务 1，随后异步统计音频文件；没有标准目录时仅只读检查数据库 schema 和路径字段，停止在第一个实际存在的音乐根目录，不加载完整歌曲记录、歌词或封面。进度事件携带 discoveryId，支持阶段切换、协作式取消、重复取消幂等和终态事件。

前端按 discoveryId 忽略迟到事件，超过 10 秒显示手动选择和取消入口，手动选择会先取消后台发现且不会被旧事件覆盖。Dashboard 数据源仍是 W4DJ 独立歌曲库。前端全量 Vitest 12 文件/202 项、TypeScript、Vite、Rust 全量测试、Tauri check、严格 Tauri Clippy、fmt 和 diff-check 已通过。最新版 arm64 App 已用 `open -g` 后台启动验收，版本仍为 `3.2.0-beta.3`；真实 10,955 条数据库的最小路径查询和辅助功能 GUI 操作仍需用户环境验收。

## 2026-08-29 输出扫描隐藏文件与歌曲库快照

扫描入口现统一使用 root-aware 隐藏路径规则：任一文件名/目录名以点开头、macOS Hidden、Windows Hidden、`.w4dj-*` 和 AppleDouble 均不会进入来源、输出、重复判断、转换候选或歌曲库。为避免 macOS `/var` 系统标志误判，系统临时 `.tmp*` 测试夹具和 `/var` 挂载标志不作为用户 Hidden 属性。任务 1 PreserveSource 只对最终点开头 stem 前缀 `_`，其他源字符和标签保持。

输出扫描现在逐项报告正常歌曲的输入/输出分母，完成扫描后按参与的输出根以事务同步 `w4dj.sqlite3` 快照；仍存在路径保留分析，消失路径及关联分析删除，空根清空，取消/失败不写库。清理入口改为清除歌曲库与分析缓存，保留音频、转换历史、歌单、偏好和 scan-cache。

验证：Rust 库 118/118、Tauri 103/103、前端 210/210、TypeScript、Vite、cargo check、fmt 和 diff-check 通过。Windows Hidden、macOS Hidden 标记及真实 1,190 首现场验收尚未执行；版本保持 `3.2.0-beta.3`，未提交或推送。

## 2026-08-29 任务 1 来源解绑与网易云状态布局

任务 1 清空来源现在会协作式取消网易云目录发现和轻量索引，清除手动数据库路径并持久化 `neteaseDatabaseBound=false`；普通来源选择不会重新绑定。显式扫描本地网易云文件夹或手动选择有效数据库会重新开启绑定，旧偏好缺少该字段时默认保持原有自动绑定行为。解绑状态跨 reload/重启保持，轻量索引文件不删除。

网易云状态已从来源工具栏移到任务 1 进度条右侧：运行中显示阶段和计数，完成显示“索引已就绪”，解绑显示“未选择数据库”。侧栏可见清理入口统一为“清除歌曲库与分析缓存”，调用 `clear_library_catalog_cache`；增强模式缓存保留独立隐藏 action。前端 211/211、Tauri 103/103、Rust 全量、TypeScript、Vite、fmt、check、严格 Tauri Clippy 和 diff-check 均通过，arm64 App 已重编译。真实 Windows/macOS 标志与用户目录现场验收仍待执行。

## 2026-08-29 统一运行日志与双层报告（实现完成，当前交接点）

共享工作树已加入 `src/runtime_journal.rs`。RuntimeJournal 在应用数据目录维护按日 JSONL，通过容量受限的 sync_channel 与独立 writer 非阻塞写入；进度事件拥塞时合并，关键事件进入最多 1000 条的内存补偿区，并按 30 天/200 MB 清理旧日志。启动写入 `app_started`，已有 active marker 时追加 `previous_run_interrupted`；日志不包含音频、封面二进制、歌词正文、数据库记录正文或凭据。

新增 Tauri 命令 `export_run_report(id,path)` 和 `export_full_runtime_report(path)`，均只在用户手动选择路径后生成结构化 JSON，先写临时文件再原子替换。历史卡片只显示“导出本次运行报告”，关于页显示“导出完整运行报告”；旧 TXT/运行会话命令仍兼容保留但不再由界面暴露。HistoryEntry 增加可选 `operationId`，来源选择、扫描、转换后台会话、网易云定位/数据库候选、取消、清理和前端运行事件已接入全局日志；报告导出前会刷新已入队事件。

本轮验证：前端 Vitest 12 文件/203 项、根 Rust 全量 120 项、Tauri 103 项、TypeScript、Vite、cargo check、fmt 和 diff-check 通过；根库与 Tauri 严格 all-targets Clippy 通过。`pnpm --dir app test -- --run` 被本机 `ERR_PNPM_IGNORED_BUILDS` 安全策略阻断，但同一依赖树直接运行 Vitest 已全部通过。RuntimeJournal 压力/轮转单元测试 4 项通过。通过 `open -g` 完成异常重启/正常退出验收：强制终止后出现 `previous_run_interrupted`，正常退出写入 `app_stopped` 并清除 active marker。最新 arm64 App 构建于 2026-08-29 18:41:42。真实扫描→转换→增强分析链路以及 Windows/Rekordbox 仍需应用级现场验收。

## 2026-08-29 运行会话路径修正

普通 App 转换的内部运行会话现在写入应用数据目录的 `W4DJ-runtime-sessions`，不再写入 Downloads。错误报告仍仅在用户手动点击导出并选择路径后生成。

## 2026-08-29 报告导出路径复核

普通转换不会自动调用报告导出，也不会向 Downloads 创建错误报告；报告动作均先由用户选择保存位置后执行。前端语义化 UI action 现在通过非阻塞日志命令写入全局 RuntimeJournal。最新 arm64 App 于 18:56:07 重新构建，并用 `open -g` 后台启动/退出烟测；Downloads 未出现新的报告文件。

## 2026-08-29 Task 1 网易云权威元数据与改编者语义（当前状态）

任务 1 预览在冲突判断前解析只读网易云身份，数据库 Title/Artist/Album、track ID 和 album ID 作为权威身份；`Original` 规则保留源 basename，其他规则才按数据库身份排列。转换、仅更新元数据、封面恢复和增强分析共享同一批次 resolver，仅补齐缺失标签，写回阶段不会再次改名。任务 2 继续使用源文件/既有 SoundCloud 清洗策略，不受网易云数据库驱动。

Mass Destruction 的全角引号与多歌手匹配、轻量 locator、无匹配/歧义回退和任务槽隔离已有回归覆盖。1,192 个真实历史路径已使用 T7 只读数据库快照完成全量文件名/身份分类：1,191 首唯一匹配，唯一未解析的 `Truth or Dare (1).ncm` 在数据库中有两个不同 track ID/专辑，正确保持歧义；117 首识别为 Remix/Edit/Version 语义。数据库大小和 mtime 前后不变，逐曲 JSON 位于 `/private/tmp/w4dj-task1-1192-acceptance-newer.json`。

合法临时 MP3、FLAC、WAV、AIFF 按单艺人、多人艺人和 Remix 三类各生成三首，12/12 通过 ffprobe；合法 FLAC 配合临时只读 SQLite 已验证 Mass Destruction 的网易云身份、标签写入和复读且源文件未改变。该验收明确区分真实文件名/NCM 派生语义层与合法临时容器层，没有使用空文件，也没有修改用户原始音频或数据库。

最新版 arm64 App 已于 2026-08-29 20:13:56 CST 重新编译，版本保持 `3.2.0-beta.3`，约 96 MB，Mach-O arm64。

## 2026-08-29 输出扫描歌曲库 UNIQUE 冲突

已修复主动输出扫描同步时的 `w4dj_track_meta.destination_path` 唯一约束失败。根因是转换登记已用 `source:/netease:` track key 占有目标路径，而旧 reconciliation 又固定构造 `output:<path>`。新实现先在事务外完整形成所有参与 root 的规范化文件快照，再用一个事务按目标路径复用既有身份；只有新文件创建 output 身份，消失文件及关联分析被删除，未参与 root 不变。

文件指纹固定为 size+mtime：两者均未变化时保留分析，任一变化或旧指纹缺失时清除 SQLite 与兼容 JSON 的旧分析并回到 `notAnalyzed`。任一参与 root 枚举/校验/同步失败时整批不提交。任务槽 DTO 使用安全的 `library_sync_failed` 代码，UI 显示“扫描成功 x/x”加红色“歌曲库同步失败”和“失败”状态；SQLite 错误、路径、旧/新身份只进入 RuntimeJournal/手动报告。

真实 `/Users/mac2/Music/test` 的 6 首已在用户 SQLite 的只读备份上验收：同步后该 root 恰好 6 条、destination 无重复、6 个旧身份均保留；第二次相同快照 `invalidatedPaths=[]`。用户数据库 size/mtime 前后相同。临时目录回归还覆盖空 root、部分替换、指纹变化、未参与 root 和两槽原子回滚。

包含该修复的最新版 arm64 App 已于 2026-08-29 21:07:39 CST 构建，版本保持 `3.2.0-beta.3`，约 96 MB。
