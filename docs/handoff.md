# W4DJ RKB 正式交接

最近更新：2026-08-25（文件名规则优先修正与五首真实样本验收）

交接日期：2026-08-23<br>
交接分支：`codex/v3.0.2`<br>
当前版本：`3.2.0-beta.3`（对外显示为 W4DJ 3.2.0 beta-3）<br>
当前开发阶段：`3.2.0 beta-3`

### 2026-08-25 文件名路由修复

已确认并已实现后续规则：用户选择的 `FilenameRule` 优先，`TitleArtist`/`ArtistTitle` 按源身份生成，`Original` 才使用源文件 basename。NUL 只在文件名中显示为 `, `，ASCII `/` 显示为全角 `／`；源标签、封面和 W4DJ 匹配不做清洗。5 首 NUL 歌手歌曲已在隔离临时目录完成真实扫描、FFmpeg 转换、目标文件名和源 Title/Artist/封面回读；10 首标题含 ASCII `/` 的批量现场回读、ExifTool 全字段报告、W4DJ SQLite 绑定和 Windows/GUI 验收仍待后续现场阶段。任务 2 的 SoundCloud 规则保持不变。

## 本次会话完成内容

本次连续开发会话基于仓库真实文件推进了《计划.md》Task 6–13，并核对了分支、工作树、源码结构、计划、已有文档、测试和 Apple Silicon 构建结果，创建/更新了：

- `/Users/mac2/Documents/W4DJ RKB/AGENTS.md`
- `/Users/mac2/Documents/W4DJ RKB/docs/project-state.md`
- `/Users/mac2/Documents/W4DJ RKB/docs/handoff.md`

在用户批准的独立验收设计基础上，本次还实现了 `Task 15` 的工作区 HTML 情绪模型验收工具；它与 Dashboard 和正式分析链路隔离。

没有推送 GitHub、合并 `main`、创建 Release、切换分支或清理用户文件；本次仅按用户要求更新版本定义。

## 当前代码包含的核心模块

当前脏工作树中的核心实现包括：`src/library_catalog.rs`、`src/library_query.rs`、`src/netease_library.rs`、`src/netease.rs`、`src/media_probe.rs`、`src/lyrics.rs`、`src/scan_cache.rs`、`src/analysis.rs`、`src/sync.rs`、`src/history.rs`，以及 Tauri 命令层 `src-tauri/src/main.rs` 和前端 `app/src/app.ts`、`app/src/library-dashboard.ts`、`app/src/analysis.ts`、`app/src/styles.css`。这些改动是当前工程状态的一部分，本次交接没有重新整理或拆分它们。

## 计划完成度

- `计划.md` Task 1–5：核心模型、只读网易云投影、媒体探测、分析投影、参数化查询已有实现和自动化覆盖，但真实数据/完整操作符覆盖仍不完整。
- Task 6：`<app-data>/w4dj.sqlite3` 已成为 Dashboard 唯一权威数据源；成功输出逐曲登记，保存实测属性、来源/目标路径、任务槽和输出根目录，分析状态按目标路径更新。A/B 根目录只在新目录成功产出后切换，旧根目录记录标记为 `outOfScope`。后台失效扫描/取消/清理命令及进度事件已接入。网易云刷新、手动数据库命令改为写独立 `library-dashboard.sqlite3` 的兼容接口，Dashboard 不调用；真实 A→B 数据与跨平台验收待补。
- Task 7：教程后不再自动发现网易云数据库；用户点击任务 1 来源标题右侧的“扫描本地网易云文件夹”后先调用 `locateNeteaseLibrary(true)`，成功返回的 `musicFolder` 自动填入任务 1，只有失败时按钮才提供“手动选择文件夹”兜底。`netease-discovery-progress` 映射到任务 1 进度条且高频更新不重建整棵 DOM；真实转换环境待验收。
- Task 7 后续交互修正：任务 1 来源标题右侧保留显式“扫描本地网易云文件夹”按钮；它不触发歌曲库刷新，WebView 右键 Reload 已禁用，页面 reload 不再重新弹教程。
- Task 8：Dashboard、事实表格、详情、歌词视图和独立输出库统计已有，前端 Dashboard 测试通过；“批量寻找失效歌曲”显示后台计数/当前文件，“清除所有失效文件”只清理 W4DJ SQLite 记录。
- 歌曲库维护补充：歌曲行右键可“重新定位文件”（保留原分析结果）或“移除记录”（只改 W4DJ SQLite）；搜索栏右侧的“清除所有失效文件”需勾选二次确认，且不删除音乐文件、网易云数据库或分析缓存。
- Task 9：列顺序/隐藏、列宽持久化、Shift 多列排序优先级、动态操作符、250ms 防抖和查询竞态防护已完成。
- Task 10：增强模式下可从 Dashboard 单独分析可读歌曲，复用分析缓存并支持取消；不调用转换和转换历史，完成后只刷新 Catalog 投影。
- 运行会话记录与错误报告：每个任务在本机应用数据目录的 W4DJ-runtime-sessions 下保留 session、候选、预检/转换/分析/模型/回写事件和逐槽摘要；这些内部记录不会写入 Downloads，也不会自动生成错误报告。转换历史中的“导出错误报告”按钮由用户手动选择路径，生成 UTF-8 文本并把导出路径回写到对应历史记录；不上传任何数据。
- Danceability 展示补充：新增 `app/src/danceability-rating.ts`，固定 S 曲线把原始值转换为 1–10 可见等级；原始 Essentia 值、SQLite/JSON、查询/排序和 Energy 不变。Joe Fight 约 1.1535 显示 6/10，缺失/非有限值显示 `—`，前端锚点和边界测试已覆盖。
- Energy 展示补充：新增 `app/src/energy-rating.ts`，按校准计划的九个 RMS² 边界显示十级星标与 `N/10`；tooltip 保留 `Essentia RMS² raw`，原始 Energy 继续用于 SQLite/JSON、筛选、排序和音频标签，Energy 筛选项仍明确使用原始数值。Danceability 与 Energy 使用独立标度。
- 增强分析卡顿修复：主线程只保留 Web Audio 解码/重采样和 MusiCNN 输入准备；新增 `analysis.worker.ts`、`analysis-worker-client.ts` 与协议模块，将同步 Essentia/WASM、MusiCNN 帧计算和 TensorFlow.js 推理移出 UI 线程。Worker 消息按 `jobId/requestId` 路由，取消立即 `terminate()`，高频进度只更新分析状态节点，不重建整棵应用 DOM。分析候选现在按 `AppPreview.slot_index` 分组，`AppAnalysisState.slotIndex` 路由到来源任务槽，任务 2 不再覆盖任务 1 的进度。增强分析结果按歌曲立即写回输出文件、W4DJ SQLite、兼容缓存和历史报告；批次中断后已完成歌曲保留，运行会话报告按歌曲累积，剩余候选可通过恢复入口继续。
- 2026-08-22 稳定性与诊断补充：MusiCNN 改为显式 `FrameGenerator` 逐帧释放并每 32 帧发送心跳；每首歌曲独立 Worker，超时按固定公式终止并继续下一首。手动“导出错误报告”现在合并转换状态、增强分析摘要、逐曲状态和运行日志，逐曲包含开始/结束时间、Worker、阶段、耗时和终止原因；另有手动“导出运行会话记录”按钮。报告不再由分析回写自动生成或覆盖，元数据诊断区分源标签实际校验与文件名推断。
- Task 11：增加 NcmCore/Enriched 元数据计划、MP3/FLAC/AIFF/WAV 字段映射、写后校验和歌词归一化接口；真实样本与 Rekordbox 可见性尚未完成。
- Task 12：私有 SQLite 损坏恢复、缓存清理边界和原始记录本机路径警示已完成；基于敏感路径外传限制，未启用将完整敏感原始 JSON 复制到系统剪贴板。
- Task 13：本地 Rust/前端/格式/Tauri/Apple Silicon App 验证完成；真实网易云、Windows、Rekordbox 和最终 DMG 验收未完成。
- Essentia/TensorFlow 已改为离线内置：MusiCNN、五个既有 Mood 头、人声/器乐头、emoMusic、MuSe、MIREX 以及 Discogs EffNet/Genre、Mood/Theme、Approachability、Instrumentation、Timbre、Danceability head 随 App 资源分发，TensorFlow.js 由 Vite 打进本地 chunk；启动自动补齐缺失或损坏副本，不依赖运行时模型下载。三个情绪 head 使用官方 ONNX 导出并通过严格校验；Discogs embedding 使用 `[64,128,96]` 输入、1280 维输出，五个新增 head 共享该 embedding，`genre_discogs400` 使用官方 400 类标签，浏览器不支持的 `PartitionedCall` 在离线准备阶段等价展开。MusiCNN 的 50 个 MSD 多标签写入 `style`，Discogs Genre 写入正式 `genre`，五个新增结果保留在 `highLevel.discogsEffnet`，不把 Style 冒充 Genre。
- Essentia 模型校验、恢复和导入命令仍留在 Rust 后端作为兼容/维护能力，但设置区已移除“恢复内置模型”“官网下载”“导入模型”三个入口，模型拖入也不再触发导入。普通界面只保留增强分析缓存和扫描缓存清理按钮，歌曲/文件夹拖入继续走原任务槽逻辑。
- 情绪模型主观验收工具已加入 `tools/emotion-evaluation/`：`export_emotion_evaluation_manifest` 从独立 W4DJ SQLite 只读抽样，优先 Drop、其次峰值能量的 10 秒片段；独立 HTML 先采集盲听情绪，再以匿名 A/B/C/D 比较 Legacy Mood、emoMusic、MuSe、MIREX，支持并列、暂停、IndexedDB 恢复和用户手动导出 JSON/CSV。静态 HTTP 服务已在本机完成启动和 curl 冒烟；浏览器主观盲听仍需人工，当前仅有 10 条实际输出，不能用重复歌曲冒充 100/200 首。旧分析记录不会因安装模型而自动补写，需重新分析后再生成正式评测批次。
- Dashboard 数据源边界已修正：网易云数据库仅在转换阶段用于元数据读取/写入，不再用于歌曲库；`library-dashboard.sqlite3` 仅保留兼容元数据暂存。`save_track_analyses` 只保存可复用的 `track-analysis.json` 镜像；目标文件完成元数据事务并通过回读校验后，`apply_track_analysis_results` 才按 `destination_path` 更新 `w4dj.sqlite3` 投影。普通转换覆盖输出时会使旧分析失效，高级模型失败时保留基础分析；Dashboard 只查询实际输出记录，并显示未分析/失败/完成统计。歌曲库工具栏提供“重新分析当前输出”，Reload 后可继续未完成候选，不触发网易云刷新。
- 独立歌曲库实现位于 `src/w4dj_library.rs`，输出登记集成在普通/增强转换安全提交之后；首次启动从成功转换历史导入实际输出，不导入网易云 database-only 记录。失效扫描通过 `library-invalid-scan-progress` 逐路径报告并支持协作取消，清理只删除 W4DJ SQLite 的失效记录及关联分析。
- 输出登记边界修复：目录登记兜底和首次历史导入都会跳过 `.w4dj-*` 临时文件、`._*` AppleDouble 文件和 `.ncm` 源文件，避免非最终产物进入 Dashboard；当前用户库中已有的临时行未被本次验收直接删除。
- 独立未来功能 `W4DJ × DJ Crate Digger` 已加入 `计划.md` 的待实现清单，详细需求保存在 `docs/W4DJ-dj-crate-digger-handoff.md`；当前为“待确认、未启动”，不得当作已授权的实现任务。

## 2026-08-23 Discogs-EffNet 五 head 交接

计划 `docs/superpowers/plans/2026-08-23-discogs-effnet-heads.md` 的代码和自动化步骤已完成。一个 `discogs_effnet_embedding` 只提取一次 1280 维 embedding，五个 head 独立执行、聚合、持久化和报告；缺失/失败/取消不会覆盖已有结果。W4DJ SQLite、`highLevel.discogsEffnet`、Dashboard 详情/筛选、手动错误报告和 `W4DJ-Discogs-*` 音频字段均已接入，新增字段不改变原始 Danceability、Genre、BPM、Key 或 MusiCNN Style。

当前已完成 154 项前端测试、49 项 Tauri 单元测试和 7 项独立歌曲库集成测试；资源合同、输入形状、输出节点、权重长度、head 隔离、重启后会话监视器、Energy 十级边界和手动报告均有覆盖。未完成的是环境相关验收：现有真实输出尚未重新跑五个 head 并用 ExifTool 回读，Energy 阈值两侧 hover/原始筛选排序也未做 GUI 人工核对，MP3/FLAC/WAV/AIFF 全格式、Windows、Rekordbox、浏览器盲听和 100/200 首人工评测仍不可由本机自动化替代。新增五个 head 的结果需用户重新分析已有输出后才会出现，安装模型不会追溯填充旧 JSON。

## 2026-08-23 分析断点恢复与性能交接

`docs/superpowers/plans/2026-08-23-analysis-resume-and-performance.md` 的代码阶段已完成。运行会话现在把真实目录写入历史，使用原子替换的 `analysis-state.json` 保存逐曲 pending/running/completed/failed/timeout/interrupted/cancelled、阶段、帧数、Worker、TensorFlow 后端/内存指标和 15 秒心跳；后端重启后会按批次懒加载持久化监视器，单例租约可拒绝并发并接管过期尝试。Reload/关闭不会自动启动分析，重新打开提供“继续未完成分析”，已完成歌曲跳过 Worker，未完成歌曲一首一个 Worker 继续；超时按 `min(15 分钟, max(5 分钟, 时长秒数×3+60秒))` 终止当前歌曲并继续，取消不会写入当前歌曲成功结果。

MusiCNN 使用连续 `Float32Array` Mel 缓冲、64 patch 固定批次、一次双输出执行和批次级 Tensor 释放；Discogs-EffNet 使用 `[64,128,96]` 流式 Mel/embedding 批次，五个 head 逐批聚合，不保存整首歌曲的 Discogs 嵌套数组。手动错误报告现在合并转换/增强分析总览、所有候选逐曲状态、最后心跳/当前阶段快照和运行日志；分析过程不会自动生成或覆盖用户报告。

本次验证：前端 8 个测试文件 156/156，Tauri 49/49，根 workspace `cargo test --all` 429 项；Vite、`cargo fmt --all -- --check`、`cargo check --manifest-path src-tauri/Cargo.toml`、Tauri `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、`git diff --check` 均通过。真实长音频三次暖运行、目标 WebView CPU/WebGL/WASM 后端实测、9 首 reload/关闭/取消/继续、Energy 阈值两侧 hover/原始筛选排序、ExifTool 标签回读、Windows/Rekordbox/最终 DMG 仍是环境相关未执行项。

## 2026-08-23 全局扫描与 FFmpeg 并发交接

已按 `docs/superpowers/specs/2026-08-23-global-scan-ffmpeg-concurrency-design.md` 实现全局扫描与转换预算。偏好字段 `concurrency_limit` 默认 2、范围 1–10，滑块和数字输入共享同一规范化入口；扫描/转换任务启动时冻结预算，两个任务槽共享 permit，运行中修改只影响下一次任务。扫描改为可取消目录枚举、固定 worker、稳定索引和协调器合并；转换改为固定 worker/受限队列，FFmpeg 通过 `spawn()` 登记到应用级 registry，取消时停止派发、杀死活跃子进程并清理未提交临时文件；扫描 worker panic 会转为可报告问题。每个槽位保留并显示本次任务的并发快照。

本轮本地自动化：根 Rust workspace `cargo test --all` 429 项通过，Tauri 49 项、前端 Vitest 156 项通过，Vite 临时目录构建、fmt、check、Tauri 严格 all-targets Clippy 和 diff-check 通过；偏好边界、共享预算、可取消等待、稳定扫描结果、扫描 worker 异常报告和 FFmpeg registry 终止已有回归。真实 M1 10 首吞吐对比、双槽真实 FFmpeg 重叠、取消 5 秒人工验收、GUI 响应、Windows/Rekordbox/最终 DMG 仍未执行，不能用本地合成测试替代。

## 最后停在哪里

最后一次工程验证完成于 Energy 十级 Dashboard 展示接入后的 Apple Silicon App 构建成功之后；当前 beta-3 App 可从以下路径打开：

`/private/tmp/w4dj-tauri-build/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`

该产物为 arm64，Info.plist 的短版本和构建版本均为 `3.2.0-beta.3`；当前内置模型资源约 25 MB（含 Discogs EffNet），由本地 bundle 携带。该路径是本地构建目录，不等于已发布的 DMG。

## 已跑过的测试

- `cargo test --all`：本次运行所有 workspace/unit/integration targets 全部通过（429 个测试用例，含 Discogs 五 head 投影/保留、并发预算和临时/AppleDouble 输出过滤回归）。
- 前端 Vitest：8 个测试文件、156 项通过（`app` 106、`analysis` 18、`analysis-worker-client` 7、`library-dashboard` 12、`danceability-rating` 4、`emotion-models` 3、`discogs-effnet` 2、`energy-rating` 4）。
- Tauri 单元测试：49 项通过（含内置模型完整性/恢复、Discogs embedding 与五个新增 head 的结构/权重/输入输出校验、ZIP 精确配对与冲突拒绝、模型导入安全性、DTO/白名单、运行会话摘要、重启后会话监视器和增量报告回归）；另有 `tests/w4dj_library.rs` 7 项通过。
- 前端生产构建：通过，有 bundle 体积提示。
- `cargo fmt --all -- --check`：通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- `cargo clippy --lib --all-features -- -D warnings`：通过。
- Rust/Tauri 带 `-A dead_code -D warnings` 的 Clippy 检查：通过。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：通过；根工作区严格 all-targets 仍保留既有 `dead_code` 基线失败。
- `cargo tauri build --target aarch64-apple-darwin --bundles app`：通过，产物版本为 `3.2.0-beta.3`。
- 本次独立输出库/Essentia/回写回归：Tauri 49 项测试通过；前端 8 个测试文件、156 项通过，包含手动错误报告/运行会话导出、逐曲超时与取消、模型启动超时、输出库统计/失效扫描控件、Worker 消息路由、可转移音频缓冲、任务 1 自动定位网易云目录与失败后手动兜底、发现进度局部 DOM 更新、hydration 竞态与重复点击锁定、双槽分析进度路由、逐歌曲分析持久化、重新分析当前输出、模型维护入口移除、移除网易云 Genre/版权/发布日期表格列、Danceability 十级曲线、Energy 十级边界/格式、三个情绪 head 和五个 Discogs head 的独立隔离回归；Vite 生产构建通过并生成独立 Worker chunk（仅已有大 chunk 警告）。
- 情绪验收工具专项：纯逻辑与页面流程 8/8 通过，`cargo test --test w4dj_library` 7/7、Tauri `cargo check`（含 CLI）、Node 语法检查和 `git diff --check` 通过；本机已启动 `python3 -m http.server 1431 --directory tools/emotion-evaluation` 并用 curl 验证页面可访问，但浏览器播放/主观盲听仍未自动完成。

## 仍需重新跑或补跑的验证

内置模型已通过 Rust 完整性/安装测试，并用本地 TensorFlow.js 对 MusiCNN、五个情绪头和人声/器乐头逐个完成实际离线加载与零输入推理；Vite 产物未发现 CDN 地址，Apple App 的 `Contents/Resources/essentia-models` 含全部七组资源。Worker 迁移、重新分析当前输出、取消不写入当前歌曲、目标文件回读校验、临时输出过滤和 Reload 恢复入口均有自动化覆盖，但仍需用真实长音频确认键盘/按钮延迟、取消时序和迁移前后分析数值一致；网易云 GUI 转换、Rekordbox、Windows 和最终 DMG 也仍待人工验收。

模型手动导入和官方网页入口已从普通界面移除；人工验收时只需确认内置模型状态正常、MP3/FLAC/文件夹拖入仍命中原任务槽，以及模型文件拖入不会改变歌曲来源。官方域名不可达不影响内置模型运行。

本次外部只读核对：真实源目录有 9 个音频，现有输出目录有 9 个最终 MP3 和一个 `.w4dj-*.wav` 临时文件；Netease SQLite 以 `mode=ro` 打开，`web_track` 可读到 `28712318/FRAGILE`，未找到 `3409113568/SHE DID IT AGAIN`。现有 9 个 MP3 的标题、歌手、专辑、时长、大小和码率可用 ExifTool/ffprobe 读回，但未把这次读取当作新一轮 GUI 转换或完整封面/歌词/Genre 验收。

## 工作树、临时文件和测试数据

- 当前有大量未提交 tracked 修改和 untracked 文件；详见 `docs/project-state.md` 的状态说明。不要 reset、checkout 或删除。
- 本次交接没有创建音频测试数据、网易云数据库副本或新的临时目录。
- 工作树中已有 `.DS_Store`、`.pnpm-store/`、本地 `W4DJ RKB.app/`、图标副本、脚本、测试报告/夹具和“macOS scripts/docs”等用户/工程文件；它们没有被本次交接删除，也不应默认加入提交。
- 构建产生的 `src-tauri/target/` 属于构建输出，通常被忽略；本地 App 路径保留供测试。

## 下一位 Codex 的接手顺序

先读：`/Users/mac2/Documents/W4DJ RKB/AGENTS.md`、`/Users/mac2/Documents/W4DJ RKB/docs/project-state.md`、本文件和 `/Users/mac2/Documents/W4DJ RKB/计划.md`。然后运行 `git status --short --branch`，确认分支和脏工作树，不要 reset 或覆盖现有改动。

## 2026-08-23 FLAC 封面数据库恢复执行结果

计划 `docs/superpowers/plans/2026-08-23-flac-cover-database-recovery.md` 的 Task 1–6 代码阶段已完成。`NeteaseMetadataResolver` 在批次开始以 SQLite read-only 加载并保留不可变记录快照；Tauri 将同一个 `Arc<ConversionMetadataContext>` 传给普通转换、仅更新元数据、重试和恢复路径，不再让每首歌曲重新猜数据库。匹配证据和封面来源进入 `MetadataDiagnostic.netease_recovery`，运行会话记录解析器摘要，手动错误报告新增 `[网易云数据库与封面恢复]` 汇总及逐曲字段。封面只走本地确定性顺序（嵌入、数据库 blob、本地引用、本地缓存、remoteOnly/missing），不访问远程 `picUrl`，已有输出封面不会覆盖。

本轮新增并通过 resolver 序列化/旧 JSON、有效/失效偏好、只读快照、数据库移除后继续使用、匹配方法、歧义、remoteOnly、数据库 blob 元数据更新和报告格式测试。自动化结果为根 workspace 468 项、Tauri 49 项、前端 jsdom 168 项；Vite、fmt、Tauri check/Clippy 和 diff-check 通过。根 workspace 严格 all-targets Clippy 仍只有既有 legacy `dead_code` 基线。当前环境没有 89 首 FLAC 源文件，因此真实 FLAC、WAL/SHM 前后、ExifTool/GUI 全链路尚未执行。

最新 App（arm64，版本 `3.2.0-beta.3`）：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`。

## 2026-08-23 手动网易云元数据数据库入口执行结果

计划 `docs/superpowers/plans/2026-08-23-manual-netease-metadata-database.md` 的 Tasks 1–5 代码阶段已完成。`NeteaseMetadataResolver::load_exact` 对手动路径严格执行 SQLite read-only 和受支持表校验；三个新 Tauri 命令负责加载状态、选择和恢复自动定位，选择/清除不启动扫描、转换、分析或 Dashboard 刷新。Task 1 UI 只在任务 1 来源标题右侧显示数据库操作，手动路径仅显示文件名并保留错误/警告状态。分析后置写回已改为批次开始解析一次数据库快照，避免逐曲重新发现数据库。

本轮定点 Rust/Tauri、history、前端 app 测试和 Vite 构建通过。真实 `sqlite_storage.sqlite3` 与代表性 FLAC/MP3 的 ExifTool 验收仍待用户提供/选择实际文件；不自动修改网易云数据库、不自动导出报告、不提交或推送。完整构建后最新 App 路径由本轮最终报告提供。

## 2026-08-24 七项计划复核结果

本轮按统筹元计划顺序复核了七项计划。代码与自动化阶段保持现状，没有发现需要覆盖已有实现的改动；新鲜验证为前端 Vitest 164/164、根 workspace `cargo test --all` 490 项、Tauri 51 项、Vite、fmt、Tauri check、Tauri all-targets Clippy 和 diff-check 全部通过。最新 arm64 App 为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`。

真实 GUI 复核启动了“继续未完成分析”，主界面从 0/9 推进到 1/9。旧构建在首曲逐曲回写阶段报出 `invalid args analyses for command apply_track_analysis_results`，所以没有把该次运行当作高级元数据通过；当前源码与前端已重新构建，需用上述最新 App 再跑一次并核对 `w4dj.sqlite3`、兼容 JSON、Genre/Style/Discogs/情绪字段及 ExifTool。当前仍未完成 89 首 FLAC/真实数据库、Windows、Rekordbox、DMG、Energy GUI、浏览器盲听和 100/200 首人工评测。

## 2026-08-24 回写类型兼容修复

已定位旧构建逐曲回写错误的边界：高级分析把不可用模型置信度保留为 JavaScript `NaN`，JSON 序列化后成为 `null`，Rust `FilteredAnalysisLabel` 原先按 `f64` 解析，导致 `apply_track_analysis_results` 拒绝整条 `analyses` 参数。现改为 `Option<f64>`，前端过滤诊断显式使用 `null`，Discogs score map 省略非有限项，并补充 Rust/前端回归测试。最新验证为前端 166/166、根 Rust 490 项、Vite、fmt、Tauri check 和 Tauri Clippy 通过；严格根 Clippy 仍保留既有 dead-code 基线。

最新 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`）。这次修复尚未用真实 9 首输出完成终态复核；当前数据库仍为 6 completed、3 failed。用户侧需手动点击“继续未完成分析”，然后检查 SQLite、`track-analysis.json` 与 ExifTool；89 首 FLAC、真实数据库、Windows、Rekordbox、Energy GUI、浏览器盲听和 100/200 首评测仍未完成。

尝试在 Node/FFmpeg 中直接对一首真实 MP3 运行整套内置模型时，进程在 TensorFlow.js CPU backend 初始化后被当前环境终止；没有写入用户数据库或音频，不能作为真实分析通过证据。正式验证仍应使用上述 App 的 WebView/Worker。

## 2026-08-23 未完成计划统筹

后续统一按 `docs/superpowers/plans/2026-08-23-w4dj-incomplete-plans-master.md` 执行。阶段 0 已完成；文件名与冲突安全阶段已完成代码和聚焦自动化：`src/filename_policy.rs` 分离可靠身份与安全文件名，源扫描保留清理后同名的不同路径，预览显式执行批次碰撞策略，跨格式输出不再无主删除，scan-cache 升级到 schema 2 并对旧 schema 安全重扫。该阶段已验证 filename policy 4 项、sync policy 112 项、preview 18 项、sync 39 项、history 12 项、NetEase 30 项、preferences 5 项和 Tauri 51 项。

下一阶段转入手动网易云数据库/FLAC 封面真实验收，随后完成断点恢复、Discogs、Energy、情绪模型和 Task 11/13 外部验收。任何缺少真实素材或平台的项目只记录为环境限制，不把自动化通过写成最终完成；当前真实 sqlite_storage.sqlite3、Mass Destruction 修复、89 首 FLAC、ExifTool/WAL/SHM、Windows、Rekordbox 和 GUI 仍未执行。

现场核对补充：现有迁移快照数据库可只读打开且 schema 受支持，但 `track` 为 0 行；当前没有可匹配的 `sqlite_storage.sqlite3` 或 89 首 FLAC。对 `Pinch - Qawwali.flac` 和已有 `Qawwali - Pinch.mp3` 做了只读 ExifTool 检查，未修改文件；源 FLAC 无可读标签，已有 MP3 有标题/艺人/专辑/封面。阶段 3 仍需用户提供真实数据库和完整素材后执行。

本轮本机验收补充：前端 jsdom 164/164、根 workspace `cargo test --all`、Tauri 51/51、Vite、check、Clippy、fmt 和 diff-check 均通过。`w4dj.sqlite3` 当前 9 条分析结果为 6 completed、3 failed，未重新跑真实输出，所以高级 genre/discogsEffnet/style 为空不能解释成模型失败。最新 App 为 `/private/tmp/w4dj-tauri-build/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，3.2.0-beta.3，内置模型资源已核对）。

推荐按需使用：代码实施、系统性调试、代码自审、完成前校验、使用 Git 工作树；只有 Actions 明确失败且用户要求修复时再使用 `github:gh-fix-ci`。下一次若需要再次交接，再使用任务交接技能。

## 2026-08-24 最新模型链复核

本轮修复了两个会直接造成“没有高级元数据”的问题：模型权重跨 Worker 传输不再先展开为巨大 `number[]`，并修正六个旧 Mood/Voice 运行时输出节点为随包图实际使用的 `model/Softmax`。新增 Tauri 资源节点回归测试；前端 Worker/分析聚焦测试 31/31 通过。

用 30 秒真实 MP3 临时片段在 Chromium Worker 中加载完整 17 个内置模型，结果为基础分析完成、Discogs Genre 有标签、五个 Discogs head 全部 `completed`、五个旧 Mood/Voice 返回结果、emoMusic/MuSe/MIREX 全部 `completed`。随后用真实长音频 `I Feel Love` 完成同一完整 17 模型链：基础分析完成、Genre 有结果、五个 Discogs head 与三套情绪 head 全部成功，耗时约 10 分钟；该验证仍只读临时 PCM，没有把结果写入用户数据库或音频，因此不能替代最新版 App 对 9 首实际输出的逐曲回写和 ExifTool 回读验收。

本轮最终自动化为前端 Vitest 167/167、根 workspace `cargo test --all` 490 项、Tauri 52 项、Vite、fmt、Tauri check、`-A dead_code -D warnings` Clippy 和 diff-check 通过；严格根 all-targets Clippy 仍只被既有 dead-code 阻断。当前仍保留的真实/人工缺口：9 首完整输出的最新 App 重分析与 SQLite/JSON/ExifTool 交叉核对、89 首 FLAC 与匹配网易云数据库、Energy 阈值 GUI、浏览器盲听、100/200 首人工评测、Windows、Rekordbox、DMG 挂载。

本轮最新 Apple Silicon App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，arm64，Info.plist `3.2.0-beta.3`，内置模型资源 38 个文件。

## 2026-08-24 后台启动验收补充

后续验收只使用 `open -g`，不调用普通 `open -a`、`activate` 或 `AXRaise`。最新 App 已用 `open -g` 启动并保持后台进程运行，未抢占前台。

## 2026-08-24 长音频内存边界修复补充

后台 `open -g` 现场复现确认旧版整首 `FrameGenerator`/Essentia 实例会让 WebContent 在 17,346 帧提取中达到约 1.6–2.4 GB，随后重建页面并回到首曲。生产实现已改为每 256 帧创建、释放并销毁一个短生命周期 Essentia 实例；帧参数、顺序、补零、进度事件和 Worker 协议不变。新增分块生命周期测试，当前前端 Vitest 为 173/173。

最终普通入口已清除临时验收脚本并重新打包 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`）。根 workspace 490 项、Tauri 52 项、TypeScript、Vite、fmt、Tauri check/Clippy、根放宽既有 dead_code 的 Clippy 和 diff-check 均通过；严格根 all-targets Clippy 仍只剩既有 dead_code。

未完成项保持如实：新分块代码尚未用最新版 App 对当前歌曲库做整库真实回写，SQLite/兼容 JSON/ExifTool、取消/恢复和 89 首 FLAC 仍需用户手动操作/真实素材；当前只读库状态为 `tracks=16`、分析结果 `8 completed/1 failed`。

本轮修复了 `app/src/analysis.ts` 中导出的 `MUSICCNN_INFERENCE_BATCH_SIZE` 与内部同名常量重复声明，并重新完成前端 Vitest 170/170、根 workspace Rust 测试、Tauri 52 项、Vite、fmt、Tauri check、Clippy 和 diff-check；最终 arm64 App 仍为 `3.2.0-beta.3`，路径为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`。

根 workspace 严格 `cargo clippy --all-targets --all-features -- -D warnings` 仍只因既有 `src/sync.rs:3378 paths_refer_to_same_file` 的 `dead_code` 失败；使用既有约定的 `-A dead_code -D warnings` 版本通过。

现场只读状态：`w4dj.sqlite3` 中当前输出目录 9 条记录，SQLite 分析状态为 7 completed、2 failed；只有 Hallelujah 的高层分析为 completed，其余旧记录含 `Can't find variable: MUSICCNN_INFERENCE_BATCH_SIZE` 的历史失败信息。尚未用最新 App 重新跑完 9 首，因此不能把 Discogs/情绪/Genre/Style 或 ExifTool 回读写成通过。后台启动不自动触发“继续未完成分析”，需要用户在界面手动点击入口后再验收。
## 2026-08-24 整库重分析授权后的只读结论

已按授权用 `open -g` 触发一次重分析；当前实现的“重新分析当前输出”只枚举任务槽输出根目录，因此本次只覆盖 `/Users/mac2/Music/test` 的 9 首，不包含独立库中 `/Users/mac2/Music/testtttt` 的另外 7 首。最终没有存活的 `w4dj-desktop` 分析进程，数据库与兼容 JSON 在连续 5 秒检查中没有新写入，SQLite 仍为 `tracks=16`、`analysis_results=8 completed/1 failed`。已有 `w4dj.sqlite3` 和 `track-analysis.json` 均保留；临时触发器已清理，下一次需要先决定“整库”是否应解除当前输出根目录过滤，再做真实回写验收。

## 2026-08-24 整库重分析候选范围已修复

`list_library_analysis_candidates` 现以 W4DJ 独立库中 `available` 且可读的本地音频为候选全集；当前任务槽输出根目录仅用于保留槽位标记，不再过滤歌曲。因此 `/Users/mac2/Music/testtttt` 等其他已登记输出也会进入重分析。Rust 回归测试已覆盖根目录外文件纳入和非音频文件排除。下一步用最新版 App 通过 `open -g` 触发 16 首真实分析，并按只读 SQLite/mtime/进程状态及最终 ExifTool 结果验收。

本轮前端 Vitest 全量 173/173，通过 Tauri `cargo check`、Vite 和 arm64 App 构建；最新产物仍为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（`3.2.0-beta.3`）。

## 2026-08-24 无 GUI 后台验收交接

用户已明确：当前 16 首和所有后续 W4DJ 验收不得打开 App GUI。`open -g` 加可访问性按钮不再是允许的验收入口，因为隐藏 WebView 可能不暴露 Dashboard 控件。统一实施计划为 `docs/superpowers/plans/2026-08-24-headless-acceptance.md`：抽取 GUI/后台共用的歌曲库分析 runner，增加隐藏 `headless.html` 运行时和 `--headless-acceptance` 命令协议，使用 JSONL、SQLite、兼容 JSON、mtime、ffprobe 与 ExifTool 验证。

16 首验收使用 `libraryAnalysis` 场景并自动执行第一首完成后的取消/恢复；后续 89 首 FLAC/数据库、四格式、Energy、Emotion、Windows 和 DMG 也迁移到后台场景。主观盲听与 Rekordbox 实机导入保留为外部人工限制，但不要求打开 W4DJ App GUI。当前只更新了计划和交接，尚未实施隐藏运行时，也没有启动分析或修改用户数据库。

## 2026-08-24 无 GUI 后台验收交接

隐藏运行时和共享 runner 已落地，测试/构建产物使用 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`）。实际 16 首验收报告为 `/private/tmp/w4dj-headless-acceptance/library-analysis-16-final.jsonl`：三次运行均在首曲 `Hallelujah - Leonard Cohen.mp3` 的 MusiCNN 17,344 帧提取末段触发 WebContent 重启，未发生 SQLite/JSON 回写；App 已正常停止，数据库保持 `8 completed / 1 failed / 7 notAnalyzed`。不要将本轮标记为分析通过，也不要删除用户数据库或 JSON；下一步定位隐藏 WebContent 的 Essentia/TensorFlow 内存或进程重启，再重新执行取消/恢复和 ExifTool 验收。

## 2026-08-24 增强分析控件临时隐藏交接

当前稳定性调试期间，`app/src/app.ts` 的
`ENHANCED_ANALYSIS_FEATURES_VISIBLE` 是增强分析 UI 的统一开关，默认值为 `false`。
它会一起隐藏增强模式选择器、Essentia 预训练模型设置和两个缓存清理按钮；这些
控件的后端命令、模型资源、Worker 与分析写回逻辑仍保留，不能据此判断增强分析后端
被删除。Tauri 启动时还会把本次运行的 `enhanced_mode` 强制设为 `false`，保证每次
启动默认使用普通模式。

调试结束要一次性恢复这组入口时，将该开关改为 `true` 后重新构建即可；不要分别
恢复旧按钮或删除后端命令。恢复后仍保持启动默认关闭，用户需要在界面中显式开启
增强模式。此约定是临时 UI 配置，后续交接和验收不得把隐藏控件误报成模型缺失。

## 2026-08-25 歌单导入、二维码与 M3U8

已实施 `docs/superpowers/plans/2026-08-24-w4dj-playlist-qr-import.md` 的代码阶段；当前活动协议已切换为全新的最小 `.w4dj` v2。v1 文件和旧字段明确拒绝，不做迁移。NetEase 纯文本/本地 QR 分页、整窗拖入、W4DJ 独立输出精确匹配与手动复核、以及通过稳定 `track_key` 解析当前路径的相对路径 M3U8 原子导出仍保留。导入/匹配/导出均不把网易云 SQLite、旧 Dashboard 投影、转换历史或分析 JSON 当作歌曲来源。

聚焦验证已通过：Rust `dj_playlist_match` 5/5、`w4dj_library` 7/7、`m3u8` 4/4，Tauri 57/57，前端 `dj-playlist` 4/4、`qr-code` 3/3、`app.test.ts` 112/112，TypeScript 与 Vite 构建通过。尚未执行真实手机扫码、网易云粘贴、10 首输出的实际匹配、M3U8 播放器或 Rekordbox 导入；这些仍需相应设备/素材。未提交、未推送、版本保持 `3.2.0-beta.3`。

2026-08-25 收尾：前端 Vitest 全量 26 个文件/380 项通过；歌单匹配、独立库、M3U8 针对性测试为 5/5、7/7、4/4，Tauri 57/57、TypeScript、Vite、`cargo check` 通过。最新 arm64 App 位于 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`。根 `cargo test --all` 的 3 个 filename fallback 失败和严格 Tauri Clippy 警告均为当前工作树既有问题；本轮未修改其行为。DMG bundler/`hdiutil` 受当前环境限制未生成新的 DMG。真实手机扫码、网易云粘贴、10 首匹配、播放器/Rekordbox 导入仍待外部验收。

随后已修复 Tauri 歌单 TXT 导出路径的未使用导入和 `collapsible_if`，严格 Tauri Clippy 与 Tauri 57 项测试再次通过；全仓 fmt 仍受既有跨文件格式差异影响，DMG 仍因当前环境的 bundler/设备限制未生成。

歌单计划继续收尾：新增“打开最近歌单”入口，从 `w4dj.sqlite3` 摘要/详情恢复持久化歌单和已有匹配；Tauri 原生拖拽 `over` 阶段保持 `.w4dj` 全窗口模糊遮罩。生产样本隔离验收确认 10 首、0 警告、386 UTF-8 字节，10 首控制输出全部唯一匹配，M3U8 10 条音频路径逐条解析为可读文件。前端全量现为 26 文件/386 测试。真实手机扫码、NetEase 粘贴、M3U8 播放器和 Rekordbox 仍未执行；arm64 App 已重建，DMG 仍受当前环境限制。

最终复跑根 `cargo test --all` 退出码为 0，先前 3 个 filename fallback 失败未重现；全仓 fmt check 的既有跨文件格式差异仍未批量改写。严格 Tauri Clippy 与 57 项 Tauri 测试通过。
## 2026-08-25 DJ playlist QR/M3U8 final manual checks

Implementation and local automated/controlled-fixture verification are complete. Remaining manual acceptance is deliberately explicit: use the latest arm64 App to scan every QR page from the supplied playlist and compare decoded text, paste the copied all-pages text into NetEase, open the exported controlled M3U8 in IINA/VLC and verify all 10 entries/order, then import it into Rekordbox and verify the same. Phone, NetEase, external-player, and Rekordbox checks were not run in this environment. The latest `.app` was built; DMG creation remains unavailable because the bundle script failed before invocation and `hdiutil` returned `设备未配置`.

Final verification update: the 100-track pagination fixture passes, frontend full run is 26 files/388 tests, Rust/Tauri/fmt/check/strict-Clippy/diff-check all pass, and the latest arm64 `.app` was rebuilt at 2026-08-25 01:00:33 with version `3.2.0-beta.3` plus the bundled FFmpeg sidecar. Phone QR, NetEase paste, external-player, and Rekordbox checks remain explicitly unexecuted; DMG creation remains blocked by the recorded bundler/device errors.

## 2026-08-25 高级模型懒加载修复

普通启动现在不扫描、校验或复制 Essentia/Discogs 模型文件；首次增强分析才调用 `ensure_essentia_models`，在 Rust 写锁内完成内置模型安装/校验，再读取模型并启动 Worker。普通转换不会加载高级模型，也不会因模型状态查询产生启动开销。前端和隐藏分析 runner 共用该入口。前端 Vitest 191/191、TypeScript、Vite、Tauri 57/57、Tauri check、严格 Tauri Clippy、Rust fmt 和 arm64 App 构建均通过。最新 App 为 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（`3.2.0-beta.3`）。

## 2026-08-25 快速数据库扫描与转换后增强分析

计划 `docs/superpowers/plans/2026-08-25-fast-database-scan-and-preconversion-analysis.md` 的实现已收口。网易云数据库发现先走 `probe_netease_database` 的只读 schema/行数探测，再通过 `load_records_from_db_observed` 以有界并发读取支持表；结果按数据库 fingerprint（路径、大小、mtime、WAL/SHM mtime）做进程内缓存复用。自动发现会先并发探测候选数据库，只加载记录数最多的支持库；手动路径先校验 schema，无效时立即回退自动发现。`locate_netease_library(true)` 现在先返回数据库/音乐目录，目录文件数通过后台 `netease-discovery-progress` 的 `checkingMusicFolder`/`completed` 事件补发，不再阻塞路径返回。

扫描与增强分析的阶段边界保持清晰：普通模式严格按“扫描 → 数据库准备 → 转换”执行，增强模式默认关闭，只有显式开启增强模式或歌曲库手动“重新分析当前输出”时，才在转换完成后进入逐曲 Worker 分析与标签写回。任务卡进度条同时承载数据库发现阶段（`locatingDatabase`、`readingRecords`、`checkingMusicFolder`）和转换后增强分析阶段；前端高频进度事件只更新局部 DOM，不重建整棵界面。

本轮新鲜验证：`cargo test --test netease` 4/4、`cargo test --test sync_policy` 115/115、`cargo test --test library_catalog` 18/18、`cargo test --manifest-path src-tauri/Cargo.toml` 57/57、`cargo fmt --all -- --check`、`cargo check --manifest-path src-tauri/Cargo.toml`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、前端 Vitest 12 文件 191/191 与 Vite 生产构建全部通过。当前 Codex 无 TTY 环境下 `pnpm --dir app test -- --run` 仍会因 `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY` / `ERR_PNPM_IGNORED_BUILDS` 中断，因此前端验证继续使用同一 `node_modules` 上的直接 `vitest`/`vite` 调用完成。

真实数据侧，本机只读核对到 `/Users/mac2/Library/Containers/com.netease.163music/Data/Documents/storage/sqlite_storage.sqlite3`：支持表为 `track` 和 `web_track`，当前计数分别为 0 和 538；read-only 探测前后 SQLite `mtime` 不变。计划中要求的“外置 T7 数据库 + 2,398 输入文件”完整时序验收在本轮仍未执行，因此外置卷 I/O、完整扫描耗时与转换起始时间仍保留为待现场验收项。未提交、未推送、版本保持 `3.2.0-beta.3`。

最终收口：扫描 worker 保持单次文件处理，任务开始额外做可取消的轻量预枚举以确定分母；取消、问题和动态总数通过 `enumerate_music_files_observed` 回报，扫描 worker 与 FFmpeg permit 隔离。当前自动化已通过前端 26 文件/390 项、根 workspace 全量 Rust、网易云/扫描/目录聚焦测试、Tauri 57 项、TypeScript、Vite、fmt、Tauri check、Tauri 严格 Clippy 和 diff-check。最新 arm64 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本 `3.2.0-beta.3`。外置 T7 与 2,398 文件不在当前环境，真实性能时序和完整 ExifTool 交叉验收仍待现场数据。
## 2026-08-25 重复歌曲专辑消歧

本轮已完成 `docs/superpowers/plans/2026-08-25-duplicate-track-disambiguation.md` 的代码和双曲真实验收。冲突判定只发生在实际生成相同目标路径的候选组内；不同专辑优先使用 `[专辑]`，同专辑再回退稳定身份/序号，普通不冲突歌曲不增加长后缀。候选保存网易云 `trackId/albumId` 和专辑，转换写回继续使用源元数据，后缀仅属于文件名。W4DJ 独立库按最终 destination path 记录身份，未修改网易云源数据库。

`STONE KOLD - Skybreak,Subten.ncm` 与 `STONE KOLD - Skybreak,Subten (1).ncm` 的真实验收通过：两条数据库记录分别为 `2707606350/STONE KOLD` 与 `2714172644/HALF BLOOD`，生成两个不同目标路径，Title/Artist/Album 与源记录一致；已有 `STONE KOLD - Skybreak, Subten.mp3` 重跑时保留，未发生覆盖。`tests/duplicate_track_acceptance.rs` 保留为挂载真实目录后可重复执行的 ignored 验收入口。

验证已完成：前端 191/191、根 Rust 全量测试、Tauri 57 项相关测试/check/严格 Clippy、TypeScript、Vite、fmt 和 diff-check 通过。最新 arm64 App 已构建于 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本保持 `3.2.0-beta.3`。尚未完成外置大批次 100 首非冲突文件名保持不变的逐文件验收和全量 ExifTool 回读。

## 2026-08-25 轻量网易云元数据缓存交接

已在共享工作树实施 `/private/tmp/W4DJ-lazy-netease-metadata-cache-handoff.md`：启动阶段改为只读网易云 schema/计数探测和 locator 缓存摘要，不再解析完整歌曲 JSON；新增 `library-dashboard.sqlite3` 的 `netease_cache_meta`/`netease_track_locators`，只保存 track ID、来源表/rowid、fingerprint、路径/文件名、大小和最小匹配键。完整记录在候选匹配后按 rowid 单曲读取，歌词/封面不进入轻量缓存。

后台命令 `prepare_netease_metadata_cache`、`cancel_netease_metadata_cache`、`load_netease_metadata_cache_status` 支持单例、进度事件、协作取消和 fingerprint 失效；事务完成前不替换旧快照。扫描取消会同步请求缓存取消，普通文件夹无网易云库时仍可继续。最新 arm64 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（3.2.0-beta.3）。

验证：前端 Vitest 392/392、TypeScript、Vite、根 cargo test、Tauri 57/57、根 check、Tauri check、fmt 和 diff-check 通过；`cargo clippy --lib -- -D warnings` 与 Tauri 严格 Clippy 通过。根严格 all-targets Clippy 仍被既有 legacy `dead_code`/test `map_identity` 阻断。外置 T7/2,398 文件冷启动和真实元数据写回尚未执行；本轮未改用户数据、未改版本、未 commit/push。

## 2026-08-25 扫描进度显示修复交接

`ScanTaskProgress` 已增加输入/输出两组独立计数。前端扫描任务卡按当前阶段显示对应目录的扫描数量；总数未知时显示“已扫描 N 项”与不定长进度条，总数确定后才显示 `N/总数`。扫描期间卡片状态为“运行中”，不会误显示“待命”。旧进度事件仍可用聚合字段回退，但会过滤掉处理数大于旧总数的不可能比例。

验证已完成：前端 392 项、Tauri 57 项、`sync_policy` 117 项及 TypeScript/Vite/Tauri check/严格 Clippy/diff-check 通过。全仓 fmt check 的既有 `netease.rs`/`netease_cache.rs` 格式差异未批量改写。最新 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`）。

## 2026-08-25 任务 1 网易云数据库命名与缺失元数据回退

已实施交接 `/private/tmp/W4DJ-task1-netease-db-filename-metadata-handoff.md`。任务 1 现在在预览生成目标路径前使用只读 `NeteaseMetadataResolver`：匹配成功时按数据库 Title/Artist、用户选择的文件名规则和 `PreserveSource` 字符策略生成一次路径；选择 `Original` 时保持源 basename。候选保存 track/album ID 及数据库身份，重复路径消歧与 W4DJ 登记可复用该身份，后续标签、封面和增强分析写回不再重命名。任务 2 的 `SoundCloud` 路由已显式保持源标签/文件名回退，不使用网易云身份补写。

任务 1 的无标签 MP3/FLAC 写回仅补缺失字段，可靠源标签优先；新增真实样本匹配回归覆盖全角/弯引号、带点标题的已知扩展名 stem、`web_track` 无本地路径的惰性 locator。聚焦 Rust：`netease` 8/8、`preview` 25/25、`sync_policy` 120/120 通过；真实 Mass Destruction FLAC 已在临时输出目录完成预览路径和 Title/Artist/Album 写后读回，源文件字节保持不变；`cargo fmt --all -- --check` 已通过。

当前仅完成只读真实数据库/源文件身份核对，没有改动网易云 SQLite、音频、既有输出或 W4DJ SQLite。真实 FLAC/MP3 重新转换、ExifTool 写后回读、外置 T7 全批次仍待用户素材/现场环境；版本保持 `3.2.0-beta.3`，未提交、未推送。
## 2026-08-25：W4DJ 最小歌单格式 v2 交接

计划：`docs/superpowers/plans/2026-08-25-minimal-w4dj-playlist-format.md`。

本轮已完成严格 v2 解析、最小字段序列化、W4DJ ID→实际输出路径匹配、转换后身份登记、UI 匹配诊断和 M3U8 重复 position 保留。协议只接受 `format/format_version=2/export_id/playlist.name/tracks.position/title/artist_display/netease_track_id`；旧 v1 或任意旧字段明确拒绝，不做兼容迁移。`netease_track_id` 按 JSON 字符串传递，没有 ID 时省略。

后续接手者只需使用新 v2 文件。匹配顺序是：W4DJ SQLite 的 ID 精确命中 →（无 ID 时）唯一标题+歌手回退 → 0 个未匹配 / 多个需确认。网易云源 SQLite 仍只读，Dashboard 和 M3U8 不从它枚举歌曲。用户可在歌单窗口手动导出新的 W4DJ v2 文件；导入旧 `uk-bass-simulated-10.w4dj` 会返回“不支持的 DJ 歌单版本：1；请重新导出全新的 v2 文件”。

自动化验收已通过：根 Rust 全量、Tauri 57 项、前端 Vitest 192 项、TypeScript、Vite、Tauri check、严格 Clippy、fmt 和 diff-check。真实手机扫码、网易云粘贴、外部播放器和 Rekordbox 导入仍待人工环境验收。版本保持 `3.2.0-beta.3`，未 commit/push/release。

最新 arm64 App：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`；已核对 Mach-O arm64 和 Info.plist `3.2.0-beta.3`。

## 2026-08-25 扫描取消与进度修复交接

任务 1 扫描现在先做可取消的输入/输出文件总数预枚举，随后任务卡按阶段显示 `processed/total`；网易云元数据匹配另有 `metadata_processed/metadata_total`。输入、输出、元数据三个阶段均检查同一取消标记，`cancel_scan` 立即返回 `cancelling`，后台结束后返回 `cancelled`，不启动转换且保留已有缓存/结果。前端扫描轮询只做任务卡局部 DOM 更新，不再每 120ms 重建根节点；取消点击先立即显示“正在取消扫描”。

网易云轻量 locator 使用路径、文件名和 stem 索引，扫描不再逐候选读取完整数据库，也不在开始时调用完整 `load_exact`。真实只读 T7 验收 `/Volumes/T7_1T/Neteast/test` 得到 1088/1088 候选与元数据事件，耗时 3.52 秒。自动化结果：Vitest 194/194、根 Rust 全量、Tauri 58/58、TypeScript、Vite、Tauri check、Tauri 严格 Clippy、fmt、diff-check 通过；根 all-targets 严格 Clippy 的 legacy dead_code 和 `duplicate_track_acceptance` map_identity 仍未处理。

最新可验收产物：`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（arm64，`3.2.0-beta.3`）。本轮未截图、未修改用户音频/数据库/分析 JSON，未提交或推送。若继续做人工验收，使用后台 `open -g`，只检查开始、关键阶段、结束三次；不要恢复反复截图轮询。

## 2026-08-27 Discogs EffNet 资源去重

App 内置包不再携带重复的旧 `discogs_effnet.{json,bin}`，只分发
`discogs_effnet_embedding.{json,bin}`；旧 ID 仍保留在导入器和运行时回退路径中，用户手动导入的
旧模型继续可用。离线生成脚本已停止复制旧副本，资源校验新增 canonical-only 断言和旧 ID 导入回归。

最新 arm64 App 约 95.5 MiB（此前约 112.9 MiB），版本 `3.2.0-beta.3`。Tauri 64 项模型相关/应用测试、根 `cargo test --all`、fmt、Clippy（Tauri target）和 diff-check 已通过；未提交、未推送。

## 2026-08-28 零启动扫描与轻量输出索引交接

本轮已实施 `/private/tmp/W4DJ-zero-startup-scan-lightweight-index-handoff.md` 的代码阶段。启动挂起根因是 Tauri setup 首次调用历史导入并在主线程遍历输出、读取音频元数据及合并网易云记录；当前 setup 只创建/打开私有 `w4dj.sqlite3` 并加载偏好，历史、输出目录、网易云歌曲表和高级模型均不在启动路径。兼容历史导入 API 未删除，但没有生产调用方。

新的 `W4djLibrary::upsert_lightweight_output` 是普通转换唯一的输出登记入口：仅使用最终安全提交已知的来源/目标、槽位、网易云 ID（若有）和最小 Title/Artist，短事务写入轻量身份、`local_files` 占位及输出映射；不 stat/probe/NCM 完整读取、不写 `output_roots` 或 `slot_output_roots`。后续同 ID 或同来源转换到 B 会更新原记录并清除旧分析，不访问或删除 A。转换后的全目录登记扫描已移除。

Dashboard 分析候选、DJ 匹配与 M3U8 重新解析全部来自 W4DJ 轻量索引，不按旧 `available/missing` 状态过滤；导出时才检查被选择的具体路径。网易云数据库和 `library-dashboard.sqlite3` 不再枚举 Dashboard 歌曲。

自动化验证已通过：根 `cargo test --all`、Tauri 101 项、Tauri check、Tauri 严格 Clippy、前端 Vitest 12 文件/198 项、TypeScript、Vite、Rust fmt 和 diff-check。兼容直转入口会传递可获得的网易云身份，输出替换清理仅允许当前输出根目录内的路径；根 workspace 严格 all-targets Clippy 仍有既有 `dead_code` 与 `tests/duplicate_track_acceptance.rs` 的 `map_identity`，在显式放宽这两项后无新增诊断。

跨用户后台启动验收已完成：使用最新 arm64 App 的临时副本并通过 `open -g` 触发，包装脚本仅用于把 `HOME/TMPDIR` 固定到隔离用户目录。全新用户 1 秒内观察到实际进程并创建空 `w4dj.sqlite3`（0 首、0 分析）；升级用户 1 秒内启动，`tracks`、`local_files`、`w4dj_track_meta`、`analysis_results`、`w4dj_output_identities` 的行内容和目标路径与生产副本一致，没有历史导入。升级副本只发生了既有 `w4dj_track_meta` 外键迁移，导致 SQLite 文件 mtime 更新，数据行未变化。辅助功能权限未授予，窗口计数为 `access-denied`；进程、临时锁和 SQLite 证据均已记录。

接手者下一步：在真实转换成功后核对只新增/更新一条轻量索引并确认分析追加到同一输出；外置 T7/2,398 文件、89 首 FLAC、Windows、Rekordbox 和人工 GUI/播放器验收仍待对应环境。验收继续使用 `open -g`，不调用 `activate`、`AXRaise`、截图或 `view_image`。

## 2026-08-28 生产包不携带验收资源

生产 Vite 配置已设置 `publicDir: false`。验收音频、验收页面和用于验收的模型链接仍
保留在工作区 `app/public`，仅由工作区验收工具按需调用，不会再复制到 `app/dist` 或
嵌入 W4DJ App。正式 Essentia 模型只从 `src-tauri/resources/essentia-models` 打包一次，
隐藏运行时入口保持不变。

最新 arm64 App 已重新构建，体积约 95 MB，版本 `3.2.0-beta.3`：
`/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`。
生产 `dist` 未发现 `acceptance-audio`、验收页面或前端模型副本；前端 198/198、TypeScript、
Vite 构建通过。未提交、未推送、未发布。

## 2026-08-28 任务 1 网易云“情况”栏（已实施）

任务 1 的网易云操作区新增固定“情况”/“Status”标签。前端 `resolveNeteaseSituation` 按错误 > 警告 > 活跃进度 > 手动数据库 > 轻量索引 > 默认提示解析状态，支持中英文、语义色、完整悬停消息和单行省略。网易云轻量缓存构建的高频事件只更新情况值，不重建整棵界面；按钮状态和终态仍触发必要重绘。现有扫描、选择数据库、恢复自动定位动作保持不变。

验证：app.test.ts 124/124、TypeScript、Vite、diff-check 通过。最新 arm64 App 仍为 `3.2.0-beta.3`，未提交或推送。

## 2026-08-28 任务 1 扫描进度与转换预览口径修复（独立 worktree）

独立 worktree：`/private/tmp/w4dj-task1-scan-preview-worktree`，分支 `codex/task1-scan-preview-20260828`。扫描阶段现在分别显示输入、输出和元数据匹配计数，完成态固定使用输入曲目分母；缓存准备进度映射到任务 1，完成快照在下一次操作前保留。预览 DTO 增加输入曲目、输出重复曲目、动态操作数量、实际数据库目录和逐曲明细；覆盖模式不会把已存在输出计入跳过。确认窗口的预计输出与可用空间在同一右侧列分两行，统计卡片可打开按 A–Z 排序的歌曲明细和安全文件打开按钮。

自动化：前端 app.test.ts 126/126、TypeScript、Vite、Tauri check、根 Rust 测试和格式检查通过。尚未在用户真实 1173 首目录执行现场扫描/覆盖转换，也未执行 GUI 文件打开验收。主工作树未被修改；未改版本号、未提交、未推送。

最终验收补充：独立 worktree 前端 Vitest 14 文件/209 项、TypeScript、Vite、根 Rust 全量、Tauri check、严格 Tauri Clippy、fmt 和 diff-check 均通过。最新 arm64 App 为 `/private/tmp/w4dj-task1-scan-preview-worktree/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（`3.2.0-beta.3`，约 95 MB）。主工作树未修改，未 commit、push、merge 或 release；真实 1173 首现场扫描/覆盖转换/文件打开链接仍待用户环境。

## 2026-08-29 扫描本地网易云文件夹后台化（已实施）

任务 1 的扫描命令现为立即返回 + 后台 worker。标准目录命中后先填入来源框，后台统计歌曲；否则只读查询支持网易云表的路径/目录字段，找到第一个存在的音乐根目录即停止，不解析完整记录。事件新增可选 discoveryId，阶段为 checkingKnownFolders、locatingDatabase、queryingPaths、checkingMusicFolder，支持 cancelling/cancelled/error 终态。

前端提供 10 秒后的手动选择和取消入口，手动选择先取消扫描，旧 discoveryId 事件会被忽略。前端全量 Vitest 202 项、TypeScript、Vite、Rust 全量测试、Tauri check、严格 Clippy、fmt 和 diff-check 已通过；最新版 arm64 App 已通过 `open -g` 后台启动验收。后续队列任务为仅按 effectivePath 调整“选择网易云数据库”可见文案。版本仍为 `3.2.0-beta.3`，未提交或推送。

## 2026-08-29 输出扫描隐藏文件与歌曲库快照（当前交接点）

已在共享工作树增量实现隐藏路径过滤、点号输出名保护、输出扫描实时计数、按输出根 reconcile 以及“清除歌曲库与分析缓存”。成功扫描才写入 W4DJ SQLite；取消、失败或无权限不改变既有库。兼容 JSON 仅在成功扫描后清理参与根中已经消失的 destination 记录，扫描缓存和历史不受影响。

自动化已通过：Rust 库 118/118、Tauri 103/103、前端 210/210、TypeScript、Vite、cargo check、fmt、diff-check。尚未完成 Windows/macOS Hidden 标志真实人工验收和 1,190 首真实目录现场验收。继续操作时保留脏工作树，不修改版本 `3.2.0-beta.3`，不 commit/push/merge/release。

## 2026-08-29 任务 1 来源解绑与网易云状态布局（当前交接点）

已实现任务 1 清空来源时取消网易云发现/索引、清除手动数据库路径、关闭绑定并持久化；普通来源选择不重绑，主动扫描或有效手动数据库选择重新绑定。绑定状态使用向后兼容偏好字段，未绑定时不自动定位数据库，轻量索引保留。

网易云状态现在位于任务 1 进度条右侧，运行显示阶段/processed/total，完成显示“索引已就绪”，未绑定显示“未选择数据库”。“清除歌曲库与分析缓存”调用歌曲库清理命令，增强缓存 action 保持隐藏独立入口。前端全量 211/211、Rust 118/118、Tauri 103/103、TypeScript、Vite、fmt、check、严格 Tauri Clippy、diff-check 和 arm64 App 构建通过。Windows/macOS Hidden 实际标志、真实用户目录和 GUI 现场验收仍待执行。

普通 App 转换的内部运行会话改存应用数据目录的 `W4DJ-runtime-sessions`，不再落入 Downloads；错误报告保持手动导出，不会自动生成。

## 2026-08-29 报告导出路径复核

普通转换只在应用数据目录写入内部运行会话和全局日志，不会自动调用报告导出，也不会向 Downloads 创建错误报告。报告只能在历史或关于页由用户选择保存路径后手动生成。前端语义化 UI action 通过非阻塞日志命令进入全局 RuntimeJournal。最新 arm64 App 于 18:56:07 重建，并用 `open -g` 后台启动/退出复核；启动期间 Downloads 未新增报告文件。

## 2026-08-29 Task 1 网易云权威元数据与改编者语义

任务 1 的输出路径现在在预览冲突判断前使用只读网易云身份（Title、Artist、Album、track ID、album ID）；`Original` 保留源 basename，其他文件名规则按数据库身份排列。转换和后续元数据/封面/增强分析写回共享批次 resolver，仅补缺失字段且不重新命名。任务 2 保持源文件策略，不能被网易云数据库驱动。Mass Destruction 的全角引号、多歌手和 NCM 派生语义已有回归覆盖。

1,192 个真实历史路径已用 T7 只读数据库快照完成全量分类：NCM 1,093、FLAC 72、MP3 27；1,191 首唯一匹配，`Truth or Dare (1).ncm` 因数据库同时存在两个不同 track ID/专辑而保持歧义，117 首归入 Remix/Edit/Version 语义。逐曲报告位于 `/private/tmp/w4dj-task1-1192-acceptance-newer.json`，数据库大小和 mtime 前后不变。合法临时 MP3/FLAC/WAV/AIFF 按三类各三首共 12 首全部通过 ffprobe；Mass Destruction 合法 FLAC + 临时只读数据库标签回写/复读通过。未使用空文件，未写用户原始音频或数据库。

最新版 arm64 App 已于 2026-08-29 20:13:56 CST 编译，版本 `3.2.0-beta.3`，约 96 MB，Mach-O arm64。

## 2026-08-29 输出扫描歌曲库 UNIQUE 冲突修复

侧会话报告的 `UNIQUE constraint failed: w4dj_track_meta.destination_path` 已用回归测试精确复现并修复。`reconcile_output_roots` 在事务外先完整构造所有参与 root 的规范化快照，事务内按 destination 复用已有 `source:/netease:` 身份；新文件才创建 output 身份，消失文件级联清理，未参与 root 不动，任一 root 失败则整批回滚。

size+mtime 同时相等才保留分析；变化或旧指纹缺失会清除 SQLite/兼容 JSON 旧分析并重置 `notAnalyzed`。同步失败被归属到受影响任务卡，保留“扫描成功 x/x”，同行红字显示“歌曲库同步失败”，右上角为“失败”，不会进入转换确认或在全局区域泄露技术错误。详细 root、destination 列表和错误链只进入 RuntimeJournal，供用户手动导出。

真实 `/Users/mac2/Music/test` 6 首已在用户库只读备份上完成两轮验收：首轮恰好 6 条、无 destination 重复、保留 6 个旧 track key；第二轮相同快照不失效任何分析。真实数据库 size/mtime 未改变。临时回归覆盖空目录、替换文件、未参与 root 和多 root 原子回滚。

最新版 arm64 App 已于 2026-08-29 21:07:39 CST 构建，版本 `3.2.0-beta.3`，约 96 MB。

## 2026-08-29 统一运行日志与双层报告（实现完成，当前交接点）

共享工作树已实现统一本地 RuntimeJournal 与两种手动 JSON 报告。日志目录为应用数据目录下的 `W4DJ-runtime-journal`，采用有界非阻塞队列、独立写入线程、进度合并、1000 条补偿区及 30 天/200 MB 轮转；启动异常 marker 会产生 `previous_run_interrupted`，不扫描旧 runtime-session 目录。转换运行会话后台事件也会同步到全局日志，报告导出前刷新队列。`export_run_report` 汇总单条转换历史、内部会话及同 operationId 的全局事件；`export_full_runtime_report` 流式导出日志、健康状态和补偿区，均通过临时文件原子替换，不自动生成或写入 Downloads。

前端转换历史只保留“导出本次运行报告”按钮；关于页新增“导出完整运行报告”。用户取消保存对话框不会调用后端。旧 `export_history_error_report`/`export_runtime_session` 命令暂留兼容但不再有可见入口。HistoryEntry 的 `operationId` 为可选字段，旧 history.json 可读取。

已验证前端 12 文件/203 项、Rust 120 项、Tauri 103 项、TypeScript、Vite、cargo check、fmt、diff-check，以及根库与 Tauri 严格 all-targets Clippy。`pnpm --dir app test -- --run` 被本机 `ERR_PNPM_IGNORED_BUILDS` 安全策略阻断，但同一依赖树直接运行 Vitest 已全部通过；RuntimeJournal 压力/轮转单元测试 4 项通过。通过 `open -g` 完成异常重启/正常退出验收：强制终止后出现 `previous_run_interrupted`，正常退出写入 `app_stopped` 并清除 active marker。最新 arm64 App 构建于 2026-08-29 18:41:42。接手者仍需在可用环境补做真实扫描→转换→增强分析链路验收；保持版本 `3.2.0-beta.3`，不 commit/push/merge/release。
