# Task 11/13 与 Danceability 外部验收及缺陷闭环实施计划

> For agentic workers: REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking. 本计划只在用户明确开始执行后实施。

> **2026-08-24 验收入口更新：** 本文尚未执行的 W4DJ GUI 操作全部由 `2026-08-24-headless-acceptance.md` 的隐藏运行时/CLI 场景取代。历史执行记录保留；Rekordbox 实机和 Windows 外部环境仍单独报告，不打开 W4DJ App GUI。

**Goal:** 在不重复实现已完成 Task 6–13 功能的前提下，完成真实网易云/音频/应用外部验收，并把已确认的 Danceability 十级 S 曲线展示改动纳入同一回归计划；只有证据暴露缺陷时才修改对应代码并补回归测试。

**Architecture:** 先以只读方式固定当前脏工作树和真实数据边界，再按“普通转换元数据 → 四种容器回读 → 增强 Worker → Danceability 展示投影 → 独立歌曲库 A/B → Dashboard 维护 → Rekordbox → Windows”顺序验收。所有结果以 W4DJ 私有 w4dj.sqlite3 和实际输出文件为准；网易云 SQLite 只在转换阶段只读使用，track-analysis.json 只保留兼容镜像。Danceability 只在前端展示层应用固定单调曲线，原始 Essentia 值、分析管线、数据库、JSON 镜像、排序和音频标签保持不变。缺陷修复按领域回到现有模块，不新建第二套歌曲库、分析缓存或目录偏好。

**Tech Stack:** Rust 2024、Tauri 2、TypeScript/Vite/Vitest、FFmpeg/ffprobe、ExifTool、SQLite、Essentia.js/TensorFlow.js Worker、macOS Rekordbox；Windows 项目检查使用已有 Rust target 和 CI 可用工具，不下载新的运行时服务。

## Global Constraints

- 保持产品版本 3.2.0-beta.3，不修改版本号、协议、模型资源或 Task 6–13 已完成接口。
- 保留共享工作树所有 tracked/untracked 改动；禁止 git reset --hard、git checkout --、批量删除和覆盖用户数据。
- 不提交、push、merge、创建 Release 或覆盖 tag；完成后只展示 git status --short 和 git diff --stat，等待用户说“定稿”。
- 网易云数据库 /Users/mac2/Library/Containers/com.netease.163music/Data/Documents/storage/sqlite_storage.sqlite3 只读打开；不得对原库执行写入、迁移、VACUUM、删除或重命名。
- 现有真实样本目录 /Users/mac2/Music/用所选项目新建的文件夹 和 /Users/mac2/Music/test 只用于用户授权的本地验收；所有新输出写入 /private/tmp/w4dj-acceptance-2026-08-19-*，不覆盖原文件。
- Dashboard、分析候选、状态统计和清理只允许读写 <app-data>/w4dj.sqlite3；不得把网易云 SQL 记录或旧 library-dashboard.sqlite3 记录重新导入 Dashboard。
- Danceability 固定使用 S(x) = 1 + 9 / (1 + exp(-4.48056 * (x - 1.10370)))，最终为 clamp(round(S(x)), 1, 10)；null、NaN 和无穷值显示缺失，不按曲库实时百分位重算。
- Danceability 只改展示标度；Energy 不使用该曲线，原始 Danceability 排序、分析结果、SQLite、JSON 镜像、音频标签和数据库 schema 不得变化。
- 不新增 hash、baseline、冻结 contract 或发布 gate；验收证据使用测试日志、SQLite 查询和用户可复核的文件读回结果。

---

### Task 1: 固定验收基线和真实输入清单

**Files:**
- Read: /Users/mac2/Documents/W4DJ RKB/AGENTS.md
- Read: /Users/mac2/Documents/W4DJ RKB/docs/project-state.md
- Read: /Users/mac2/Documents/W4DJ RKB/docs/handoff.md
- Read: /Users/mac2/Documents/W4DJ RKB/计划.md
- Modify only if evidence requires: docs/project-state.md, docs/handoff.md

**Interfaces:**
- Consumes: 当前分支、工作树、已有测试夹具和用户本地数据库。
- Produces: 不改变代码的验收基线，包括输入文件清单、初始输出数量、初始 W4DJ 数据库路径和待验收项。

- [x] Step 1: 记录工作树和版本，不清理现有改动

Run:

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB"
git status --short --branch
git branch --show-current
git diff --stat
git diff --check
~~~

Expected: 分支仍为 codex/v3.0.2，版本仍为 3.2.0-beta.3，工作树保持脏状态；git diff --check 不输出错误。

- [x] Step 2: 读取真实输入但不写入数据库

Run:

~~~bash
find "/Users/mac2/Music/用所选项目新建的文件夹" -maxdepth 1 -type f -print | sort
find "/Users/mac2/Music/test" -maxdepth 1 -type f \( -iname '*.mp3' -o -iname '*.flac' -o -iname '*.wav' -o -iname '*.aiff' -o -iname '*.aif' -o -iname '*.ncm' \) -print | sort
sqlite3 'file:/Users/mac2/Library/Containers/com.netease.163music/Data/Documents/storage/sqlite_storage.sqlite3?mode=ro' ".tables"
~~~

Expected: 真实数据库可只读打开；记录 FRAGILE（track id 28712318）和 SHE DID IT AGAIN（track id 3409113568）是否存在；原始音频目录与输出目录的文件数量分别记录，不把这些路径写入产品数据。

- [x] Step 3: 建立可回收验收输出目录

Run:

~~~bash
acceptance_root="$(mktemp -d /private/tmp/w4dj-acceptance-2026-08-19-XXXXXX)"
mkdir -p "$acceptance_root/normal" "$acceptance_root/enriched" "$acceptance_root/root-a" "$acceptance_root/root-b"
printf '%s\n' "$acceptance_root" > /private/tmp/w4dj-last-acceptance-root
~~~

Expected: 只创建一个明确的临时根目录；后续转换全部使用该根目录的子目录，原始网易云目录和 /Users/mac2/Music/test 不被覆盖。

---

### Task 2: 真实网易云普通转换与元数据链路

**Files:**
- Modify only if a failing assertion identifies a defect: src/netease_library.rs, src/metadata.rs, src/sync.rs, src-tauri/src/main.rs
- Test when modified: tests/library_catalog.rs, tests/desktop_flow.rs, tests/task_state.rs

**Interfaces:**
- Consumes: Task 1 的只读网易云数据库、NCM/音频输入和 MetadataWriteProfile::NcmCore。
- Produces: 临时输出目录中的普通转换结果、标签读回证据和 W4DJ notAnalyzed 记录；不启动增强分析。

- [ ] Step 1: 用 App 任务 1 选择真实网易云来源并关闭增强模式

操作：在新构建 App 中选择 /Users/mac2/Music/用所选项目新建的文件夹 为任务 1 来源，选择验收根目录的 normal 子目录为输出，保持普通模式，执行转换；若 App 要求数据库定位，只选择 /Users/mac2/Library/Containers/com.netease.163music/Data/Documents/storage/sqlite_storage.sqlite3，确认界面显示只读用途。验收根目录从 /private/tmp/w4dj-last-acceptance-root 读取，不手写固定临时目录。

Expected: 转换完成后每个成功输出都登记到 W4DJ SQLite，分析状态为 notAnalyzed；Dashboard 不出现 database-only 曲目，不自动触发网易云刷新。

- [ ] Step 2: 用 ExifTool/ffprobe 读取身份标签和技术属性

Run:

~~~bash
acceptance_root="$(cat /private/tmp/w4dj-last-acceptance-root)"
find "$acceptance_root/normal" -type f -print0 | xargs -0 exiftool -S -Title -Artist -Album -Genre -Copyright -Date -Lyrics
find "$acceptance_root/normal" -type f -print0 | xargs -0 ffprobe -v error -show_entries format=format_name,duration,size,bit_rate -of json
~~~

Expected: FRAGILE 与 SHE DID IT AGAIN 的歌曲名、完整歌手列表、专辑和封面与数据库匹配；曲目 ID、专辑 ID 不出现在用户可见标签；格式、大小、码率、时长来自输出实测。

- [ ] Step 3: 核对 W4DJ 投影和数量边界

使用 Dashboard 或只读 SQLite 查询 <app-data>/w4dj.sqlite3 的 tracks、local_files、analysis_results，确认每个输出 destination_path 唯一、状态为 available、分析为 notAnalyzed，并确认网易云数据库中的 database-only 记录没有新增为 Dashboard 歌曲。

Expected: Dashboard 总数等于实际成功提交的输出文件数，而不是网易云 SQL 记录数；任何数量或绑定错误都先写成失败证据，再进入后续最小修复。

---

### Task 3: 四种容器元数据回读与歌词交付

**Files:**
- Modify only on failing evidence: src/metadata.rs, src/lyrics.rs, src/sync.rs
- Test when modified: tests/library_catalog.rs

**Interfaces:**
- Consumes: Task 2 的身份元数据、歌词和实际输出文件。
- Produces: MP3/FLAC/AIFF/WAV 的 written_fields、unsupported_fields、回读结果和同名 .lrc 文件。

- [ ] Step 1: 抽取四种格式样本并分别读回

对现有真实音频和转换结果逐格式检查：MP3/AIFF 读 ID3，FLAC 读 Vorbis Comment 与图片块，WAV 读 RIFF INFO；同时使用 ffprobe 检查容器格式。缺失格式时只记录环境缺口，不伪造通过结果。

Expected: 每种格式至少有一份成功回读样本；歌曲名、歌手、专辑、Genre、日期、版权和封面按计划映射，WAV 不依赖私有不可见 chunk。

- [ ] Step 2: 验证歌词 Dashboard 与 sidecar

在 Dashboard 详情抽屉打开歌词页签，分别查看原文、翻译、罗马音，执行搜索、复制和下载；对有时间轴的记录检查同名 .lrc，对无歌词的记录检查空状态。

Expected: Dashboard 不向网络请求歌词；.lrc 与音频同名、写入失败不删除音频，已有不同内容 sidecar 不被静默覆盖。

- [ ] Step 3: 只在失败时补最小回归修复

若任一格式读回失败，先在 tests/library_catalog.rs 增加该格式的最小失败夹具和字段断言，再只修改对应容器分支；若歌词失败，只修改 normalize_lyrics/write_lyrics_sidecar 路径。修复后运行：

~~~bash
cargo test --test library_catalog metadata
cargo fmt --all -- --check
~~~

Expected: 失败场景先可重现，修复后测试通过；不得通过放宽断言、把数据库 ID 写入标签或把 Essentia 值覆盖身份标签来“通过”。

---

### Task 4: 增强 Worker、取消和数值链人工验收

**Files:**
- Modify only on failing evidence: app/src/analysis.ts, app/src/analysis.worker.ts, app/src/analysis-worker-client.ts, app/src/app.ts
- Test when modified: app/src/analysis.test.ts, app/src/analysis-worker-client.test.ts, app/src/app.test.ts

**Interfaces:**
- Consumes: Task 2 的实际输出文件和 w4dj.sqlite3 未分析记录。
- Produces: Worker 进度/取消/成功结果、目标输出绑定的分析投影和不冻结 UI 的人工证据。

- [ ] Step 1: 在真实输出上启动增强分析并记录进度

在 Dashboard 选择“重新分析当前输出”，记录当前歌曲、阶段、计数、Genre/BPM/Key/Loudness 等结果；在分析期间操作搜索、任务槽切换和窗口按钮，观察是否出现多秒级输入延迟或整页闪烁。

Expected: 进度持续更新，普通按钮/键盘保持可响应；任务 2 的进度只在任务 2 卡片显示；Worker 失败不回退到主线程同步分析。

- [ ] Step 2: 中途取消并验证投影不污染

在一首歌曲 Worker 正在计算时点击“取消分析”，然后立即再次打开 Dashboard；检查当前歌曲没有写入新的完成结果，先前已完成歌曲仍保留，任务可再次启动。

Expected: Worker 被终止，旧 jobId 消息被忽略，W4DJ analysis_results 不出现半成品；取消后无需 reload 才能操作其他按钮。

- [ ] Step 3: 验证成功写回和失败降级

成功分析一首输出后用 ExifTool/ffprobe 和 Dashboard 详情确认分析结果绑定 destination_path；让一个高级模型缺失或故意失败，确认基础能量/可舞性等结果仍保留，失败状态不会覆盖已有完成结果。

Expected: track-analysis.json 仅为兼容镜像，Dashboard 读 W4DJ SQLite；目标文件回读校验失败时不得标记分析完成。

- [ ] Step 4: 只在失败时补 Worker 回归测试

若出现输入延迟、旧消息污染、取消后写入或数值链变化，先在对应前端测试中固定失败时序，再修改 Worker 协议实现；保持 start/progress/result/error 消息和 jobId 字段不变。运行：

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB/app"
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner
~~~

Expected: 4 个测试文件全部通过；不得把 Worker 错误静默转回主线程计算。

---

### Task 5: Danceability 十级 S 曲线展示改动

**Files:**
- Create: app/src/danceability-rating.ts
- Create: app/src/danceability-rating.test.ts
- Modify: app/src/library-dashboard.ts
- Modify: app/src/library-dashboard.test.ts
- Modify after implementation: 计划.md, docs/project-state.md, docs/handoff.md

**Interfaces:**
- Produces: danceabilityLevel(value: number | null): number | null。
- Produces: formatDanceabilityRating(value: number | null): string。
- Preserves: Dashboard 查询 DTO、SQLite/JSON 中的原始 Danceability、排序字段和 Energy 的现有展示。

- [x] Step 1: 先写固定锚点、边界和数学性质测试

在 app/src/danceability-rating.test.ts 固定以下断言：原始值 0.8240978122 映射 3，Joe Fight 的 1.1535 映射 6，Friday Night 的 2.8114326 映射 10；null、NaN、正负无穷显示缺失；有限极端值被限制在 1–10；一组递增输入得到非递减等级；formatDanceabilityRating(1.1535) 为 `★★★ 6/10`，缺失为 `—`。

- [x] Step 2: 运行聚焦测试确认模块缺失或旧展示失败

Run:

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB/app"
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner src/danceability-rating.test.ts src/library-dashboard.test.ts
~~~

Expected: 在模块尚未创建或 Dashboard 仍使用 raw / 3 展示时，测试失败；不得先修改实现再补“通过”断言。

- [x] Step 3: 实现独立固定曲线模块

在 app/src/danceability-rating.ts 导出常量 DANCEABILITY_CURVE_SLOPE = 4.48056、DANCEABILITY_CURVE_MIDPOINT = 1.10370、danceabilityLevel 和 formatDanceabilityRating。映射必须先检查 Number.isFinite，再计算 `1 + 9 / (1 + Math.exp(-slope * (value - midpoint)))`，最后 round 并 clamp 到 1–10；格式化显示半星级文本和等级，不把原始值改写。

- [x] Step 4: 只替换 Dashboard 的 Danceability 展示

在 app/src/library-dashboard.ts 导入 formatDanceabilityRating；表格和详情使用十级格式，显示列名改为“可舞性（10级）”，原始值筛选选项改为“可舞性原始值”，title 保留 `Essentia raw: <value>`。Energy 继续使用原有数值展示；不得改查询字段、操作符、排序字段、分析算法或回写路径。

- [x] Step 5: 运行 Danceability 与 Dashboard 回归

Run:

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB/app"
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner src/danceability-rating.test.ts src/library-dashboard.test.ts
~~~

Expected: Joe Fight 显示 6/10，Friday Night 显示 10/10，缺失值显示 `—`，Energy 视觉和排序不变；Dashboard 测试通过。

- [x] Step 6: 验证原始值和排序未被展示层改写

用 Dashboard 查询同一批歌曲，比较改动前后的原始 Danceability、SQLite analysis_results、track-analysis.json 和排序顺序；检查打开 Dashboard、筛选和排序没有写音频文件或数据库。

Expected: 只有可见格式变化；原始数值、排序和所有存储层字节内容保持不变。不要把 1,245 首分布占比写成 baseline、hash、冻结 contract 或发布 gate。

---

### Task 6: 独立歌曲库 A/B 根目录和失效维护

**Files:**
- Modify only on failing evidence: src/w4dj_library.rs, src/sync.rs, src-tauri/src/main.rs, app/src/library-dashboard.ts
- Test when modified: tests/w4dj_library.rs, tests/library_catalog.rs, app/src/library-dashboard.test.ts

**Interfaces:**
- Consumes: Task 2/4 的成功输出与分析结果。
- Produces: A/B 根目录“成功使用后生效”的数据库状态、失效扫描/清理结果和 Dashboard 统计。

- [ ] Step 1: 验证只改输出偏好不改数据库

将任务 1 输出从验收根目录的 root-a 子目录改为 root-b 子目录，不执行转换，比较前后 w4dj.sqlite3 的 tracks、output_roots 和 slot_output_roots 查询结果；验收根目录从 /private/tmp/w4dj-last-acceptance-root 读取。

Expected: 数据库不变化；第一次在 B 成功输出后才切换任务槽应用根目录，A 仍被另一任务使用时不被标记失效。

- [ ] Step 2: 验证失效扫描和清理边界

对一个已登记输出做可恢复的临时移动，在 Dashboard 点击“批量寻找失效歌曲”，再恢复文件；随后用另一个临时记录验证“一键清除失效歌曲”。

Expected: 扫描只更新 missing/unreadable/outOfScope 状态，不删除音频；清理只删除 W4DJ SQLite 记录及关联分析，不改网易云数据库、转换历史、scan-cache.json 或模型文件。

- [ ] Step 3: 验证重新定位保留分析

将一条已分析记录绑定到同内容的备用输出路径，执行“重新定位文件”，比较前后 analysis_results 和 Dashboard 完成状态。

Expected: 只更新本地文件绑定，分析投影保留；“移除记录”只删除 W4DJ SQLite 记录，不删除音频。

---

### Task 7: Rekordbox 和 Windows 外部环境验收

**Files:**
- Modify only if external evidence identifies a portable-path or tag defect: files listed in Tasks 2–5
- Update after evidence: 计划.md, docs/project-state.md, docs/handoff.md

**Interfaces:**
- Consumes: Task 3 的四格式产物、Task 6 的实际输出库、Apple Silicon App。
- Produces: Task 11 Step 7 和 Task 13 Steps 4–5 的可追溯验收结论。

- [ ] Step 1: 在 Rekordbox 导入四种样本

把 MP3、FLAC、AIFF、WAV 输出和相对路径播放列表导入当前 macOS Rekordbox，逐条检查歌曲名、歌手、专辑、Genre、Comments、封面、时长和波形；歌词只按 Dashboard/sidecar 验收，不要求 Rekordbox 显示歌词。

Expected: 记录每种格式的实际可见字段和任何差异；不把“文件能播放”当作标签可见性通过。

- [x] Step 2: 验证 Windows target 或明确记录环境阻塞

Run:

~~~bash
rustup target list --installed
cargo check --target x86_64-pc-windows-msvc --manifest-path src-tauri/Cargo.toml
~~~

Expected: 若 target 和离线依赖可用则通过；若当前 macOS 缺少 Windows target/SDK/依赖，记录确切错误和后续人工步骤，不修改跨平台代码猜测通过。

- [ ] Step 3: 验收 Task 1/2 跨平台来源行为

在 Windows 环境至少检查任务 1/2 文件夹选择、单曲父目录打开、%APPDATA%/%LOCALAPPDATA% 配置路径和输出文件名大小写；确认教程/reload 不触发网易云发现。

Expected: 记录人工截图/日志；没有 Windows 环境时保留未完成状态。

---

### Task 8: 全量回归、文档状态和交付摘要

**Files:**
- Modify: 计划.md, docs/project-state.md, docs/handoff.md
- Modify code only if Tasks 2–7 produced a reproducible failure and its regression test is green.

**Interfaces:**
- Consumes: Tasks 1–7 的日志、查询结果、人工验收记录和修复测试。
- Produces: 可执行验收结论；Task 11 Step 7、Task 13 Step 4/5 只有证据完整时标记 [x]。

- [x] Step 1: 运行当前完整自动化验证

Run:

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB"
cargo test --all
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --lib --all-features -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -A dead_code -D warnings
cd app
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vite/bin/vite.js build --config vite.config.ts --configLoader runner --outDir /private/tmp/w4dj-vite-build-external-acceptance
~~~

Expected: Rust、前端、Vite 和格式检查通过，且包含 danceability-rating.test.ts；严格根 workspace Clippy 的既有 dead_code 只能如实记录，不得改写成全量通过。

- [x] Step 2: 更新状态文档和计划复选框

只将已实际完成的 Task 11 Step 7、Task 13 Step 4/5 勾选为 [x]；在三份文档中记录实际样本数量、Rekordbox 字段结果、Windows 环境限制和未执行项目。若证据不足，保留 [ ] 并写出人工验收步骤。

- [x] Step 3: 展示工作树并等待定稿

Run:

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB"
git diff --check
git status --short --branch
git diff --stat
~~~

Expected: 不提交、不 push、不修改版本号；向用户报告各项自动化/人工验收结果、仍受环境限制的项目、工作树状态和 diff 摘要，等待用户确认。

### 本次执行记录（2026-08-19）

- 只读基线：真实源目录有 9 个可转换音频，现有 `/Users/mac2/Music/test` 有 9 个最终 MP3，另有一个 `.w4dj-*.wav` 临时文件；Netease SQLite 可只读打开，`FRAGILE`（28712318）可从 `web_track` 读到，`SHE DID IT AGAIN`（3409113568）在当前数据库没有对应记录。
- 发现并修复了输出登记兜底会把 `.w4dj-*.wav` 临时文件登记进 W4DJ SQLite 的缺陷；目录登记和历史首次导入现在都会跳过 `.w4dj-*`、`._*` 和 `.ncm`，并补充 Rust 回归断言。现有用户数据库未被修改，已有临时行需用户在 Dashboard 失效清理/重新登记后处理。
- Danceability 十级展示已实现：固定 S 曲线只作用于 Dashboard 展示，Joe Fight 原始值约 1.1535 显示 6/10；原始值、查询字段、排序、Energy 和存储层保持不变。`Friday Night` 锚点由固定数学测试覆盖，但当前真实输出/镜像中没有该曲。
- 自动化：Rust workspace 401/401、Tauri 39/39、前端 5 个测试文件 133/133、Vite 构建、`cargo fmt --all -- --check`、Tauri `cargo check`、库级 Clippy 与 Tauri `-A dead_code` Clippy 均通过；根 workspace 严格 all-targets Clippy 仍被既有 44 个 `dead_code` 报告阻断。
- Apple Silicon App 已重建：`/private/tmp/w4dj-tauri-external-acceptance/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，Info.plist 为 `3.2.0-beta.3`、arm64、内置模型资源 15 个；验收副本使用 ad-hoc 签名通过 `codesign --verify --deep --strict`，不代表正式签名。
- 尚未执行：真实 GUI 转换/增强分析、A→B 与失效扫描人工时序、四容器完整样本、Rekordbox、Windows 和最终 DMG 挂载。当前环境没有 `rustup`/Windows target，Windows 检查因离线依赖缺失及代理不可达失败；这些步骤保留人工验收，不伪造通过。

## Acceptance Criteria

- 真实网易云转换的身份标签、封面、歌词和 Genre 匹配有 ExifTool/ffprobe/SQLite 证据，且 database-only 记录不进入 Dashboard。
- MP3、FLAC、AIFF、WAV 均有实际标签回读；不支持字段明确记录；同名 .lrc 行为符合计划。
- 增强分析期间 UI 可操作，Worker 进度/取消/失败降级和 destination_path 写回均有真实操作或自动化证据。
- Danceability 原始值保持不变，固定 S 曲线把 Joe Fight 1.1535 显示为 6/10、Friday Night 2.8114326 显示为 10/10；缺失/非有限值显示 `—`，Energy、排序、筛选和存储层不受影响。
- A/B 输出根目录、失效扫描、清理和重新定位不越过 W4DJ SQLite 数据边界。
- Rekordbox 实际可见字段和 Windows 检查结果如实记录；环境缺失不伪造通过。
- 全量自动化验证通过后，计划和交接文档与实际计数一致；未提交、未 push、未发布。

## Known Limits

当前 macOS 环境不能替代 Windows SDK、Windows Finder/Explorer 或 Rekordbox 的所有平台行为；若缺少这些条件，只完成可执行的本地验证并保留人工步骤，不因此修改产品需求或引入跨平台猜测代码。Danceability 的 1,245 首成功/60 首失败扫描只用于选择固定展示锚点，不是运行时曲库配额、baseline 或发布门禁。

## 2026-08-24 验收入口迁移

本计划的后续机器验收改由 `2026-08-24-headless-acceptance.md` 的隐藏场景执行；Windows、Rekordbox 等外部人工边界仍单独记录，不打开 W4DJ GUI。
