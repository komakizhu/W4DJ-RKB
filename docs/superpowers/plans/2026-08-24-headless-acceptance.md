# W4DJ 无 GUI 后台验收实施计划

> **For agentic workers:** 按任务顺序实施并逐项验证。用户已明确要求后续验收不得打开 W4DJ App GUI；本计划不授权提交、推送、合并、发布或修改版本号。

**Goal:** 建立可重复的无窗口后台验收入口，并用它完成当前 16 首歌曲及后续真实数据库、FLAC、分析模型、Energy、外部格式和安装包验收。

**Architecture:** 把现有歌曲库分析编排从页面事件中抽成共享 runner；普通界面和隐藏验收页面调用同一 runner。Tauri 通过命令行参数启动隐藏 WebView，只承载现有 Web Audio、Essentia/WASM、TensorFlow.js Worker 和模型资源，不创建可见窗口、不依赖可访问性按钮。进度和终态以 JSONL、SQLite、兼容 JSON、文件 mtime 和 ExifTool 为证据。

**Tech Stack:** Rust/Tauri、TypeScript/Vite、Web Audio、Web Worker、Essentia.js/WASM、TensorFlow.js、SQLite、JSONL、ExifTool、ffprobe。

## Global Constraints

- W4DJ GUI 在所有自动化和真实数据验收中保持不可见；禁止 `open -a`、`activate`、`AXRaise`、可访问性点击、坐标点击、`screencapture` 和 `view_image`。
- 允许启动隐藏的 App/WebView 分析运行时，因为现有解码、WASM 和 TensorFlow Worker 依赖该运行时；隐藏运行时不得调用 `show`、`focus` 或抢占前台。
- 当前产品版本保持 W4DJ `3.2.0 beta-3`，代码 SemVer 保持 `3.2.0-beta.3`。
- 不删除或重建 `w4dj.sqlite3`、`track-analysis.json`、已有分析结果或用户音频；取消和失败保留已完成结果。
- 网易云数据库始终只读；不下载远程封面，不自动生成或覆盖用户错误报告。
- 不新增 hash、baseline、冻结 contract 或发布 gate。
- 主观盲听和 Rekordbox 实机导入不能伪造成后台自动验收；后台只准备可复核素材和机器证据，缺少人工或外部平台时明确记录限制。
- 每次实施完成后运行测试、构建和格式检查，报告 `git status`、`git diff --stat` 及最新 App 链接；不 commit 或 push。

---

### Task 1: 建立隐藏验收运行时与稳定命令协议

**Files:**

- Create: `app/headless.html`
- Create: `app/src/headless-acceptance.ts`
- Create: `app/src/headless-acceptance.test.ts`
- Modify: `app/vite.config.ts`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/main.rs`

**Interfaces:**

```ts
export type HeadlessAcceptanceScenario =
  | 'libraryAnalysis'
  | 'neteaseMetadata'
  | 'flacCoverRecovery'
  | 'energyDashboard'
  | 'emotionManifest'
  | 'externalFormats'
  | 'bundleSmoke';

export type HeadlessAcceptanceRequest = {
  runId: string;
  scenario: HeadlessAcceptanceScenario;
  scope?: 'available';
  exerciseCancelResume?: boolean;
  inputPath?: string;
  outputPath?: string;
  databasePath?: string;
  reportPath: string;
};

export type HeadlessAcceptanceEvent = {
  runId: string;
  scenario: HeadlessAcceptanceScenario;
  status: 'starting' | 'running' | 'cancelling' | 'resuming' | 'completed' | 'partial' | 'blocked' | 'error';
  stage: string;
  processed?: number;
  total?: number;
  currentItem?: string;
  message?: string;
  timestampMs: number;
};
```

- [x] 增加参数解析：`--headless-acceptance <scenario>`、`--acceptance-report <absolute-jsonl-path>`、`--exercise-cancel-resume`、`--input`、`--output`、`--database`。
- [x] 参数存在时仅创建 `visible: false` 的 `headless.html` WebView；不得创建主窗口、Dock 激活或调用窗口显示/聚焦 API。
- [x] `headless.html` 只初始化验收 runner，不渲染 Dashboard，不绑定教程、歌曲库或任务卡按钮。
- [x] JSONL 逐事件追加写入显式路径；每行包含 `runId/scenario/status/stage/timestampMs`。退出码固定为：`0=通过`、`2=部分失败`、`3=环境阻塞`、`4=内部错误`。
- [x] 无参数时保持现有 App GUI 行为完全不变。
- [x] 测试隐藏窗口配置、参数校验、路径必须绝对、未知场景拒绝、JSONL camelCase 和退出码映射。

**验证：**

```bash
cargo test --manifest-path src-tauri/Cargo.toml headless_acceptance
pnpm --dir app test -- --run app/src/headless-acceptance.test.ts
```

---

### Task 2: 抽取 GUI 与后台共用的歌曲库分析 runner

**Files:**

- Create: `app/src/library-analysis-runner.ts`
- Create: `app/src/library-analysis-runner.test.ts`
- Modify: `app/src/app.ts`
- Modify: `app/src/headless-acceptance.ts`
- Modify: `app/src/analysis-worker-client.ts`

**Interfaces:**

```ts
export type LibraryAnalysisRunOptions = {
  runId: string;
  candidates: LibraryAnalysisCandidate[];
  resumeIncomplete: boolean;
  cancelAfterNewCompleted?: number;
  onEvent: (event: HeadlessAcceptanceEvent) => void;
};

export type LibraryAnalysisRunResult = {
  total: number;
  completed: number;
  failed: number;
  timedOut: number;
  cancelled: number;
  pending: number;
};

export function runLibraryAnalysis(
  options: LibraryAnalysisRunOptions,
): Promise<LibraryAnalysisRunResult>;
```

- [x] 把 `reanalyzeLibrary` 中候选读取、逐曲 Worker 生命周期、逐曲回写、超时继续、取消和恢复编排移到共享 runner；GUI 只负责把 runner 事件映射到现有界面。
- [x] 后台场景调用 `list_library_analysis_candidates`，候选必须覆盖 `w4dj.sqlite3` 中全部 `available` 且可读音频，不读取 Dashboard DOM 或任务槽当前目录决定范围。
- [ ] `--exercise-cancel-resume` 在第一首“新完成结果”持久化后请求取消，确认当前歌曲结束或取消终态后，从现有未完成会话恢复；不能重新运行已完成歌曲。
- [x] 每首歌仍使用独立 Worker；取消、错误和完成均销毁 Worker。禁止后台失败后回退到主线程分析。
- [x] 测试 GUI/后台调用同一 runner、16 首候选范围、第一首后取消、恢复跳过已完成、旧 runId 事件过滤、部分失败退出码和无候选终态。
- [ ] 第一首完成后的真实取消/恢复仍需 WebContent 不重启后再验收；本轮被隐藏 WebContent 重启阻断。

**验证：**

```bash
pnpm --dir app test -- --run app/src/library-analysis-runner.test.ts app/src/app.test.ts
```

---

### Task 3: 当前 16 首无 GUI 整库验收

**Files:**

- Read-only: `<app-data>/w4dj.sqlite3`
- Read-only/compatibility write by production flow: `<app-data>/track-analysis.json`
- Read/write by existing production flow: 16 个 `available` 音频的 W4DJ 分析标签
- Create runtime report only: `/private/tmp/w4dj-headless-acceptance/<run-id>.jsonl`

- [x] 只读记录 arm64、`3.2.0-beta.3`、16 首可读性和 `8 completed / 1 failed / 7 notAnalyzed` 基线；不得创建 baseline/hash 文件。
- [x] 直接运行 App bundle 内二进制，不调用 `open`：

```bash
"/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app/Contents/MacOS/w4dj-desktop" \
  --headless-acceptance libraryAnalysis \
  --exercise-cancel-resume \
  --acceptance-report /private/tmp/w4dj-headless-acceptance/library-analysis-16.jsonl
```

- [x] 每 60 秒只读记录 App/WebContent/Worker CPU 与内存、SQLite 状态数量、最大 `analyzed_at_ms`、SQLite/JSON mtime 和 JSONL 最后一条事件。
- [ ] 两分钟内没有进程活动、JSONL 事件或持久化变化则以退出码 4 停止；单首 25 分钟无进展时请求协作取消，保留已有结果并返回部分失败。
- [ ] 验证取消前完成结果保留、当前歌曲不被误标成功、恢复后跳过已完成并继续剩余歌曲。
- [ ] 完整通过要求 16/16 `analysis_results.status=completed`、`highLevel.status=completed`、有限 BPM/Key/LUFS/Energy/Danceability、非空高级 Genre、Discogs 五 head 和 emoMusic/MuSe/MIREX 均完成。
- [ ] 精确匹配 16 个 `destination_path` 比较 SQLite 与 `track-analysis.json`；兼容 JSON 可保留历史路径，总长度无需等于 16。
- [ ] 用 ExifTool 回读 16 首的 BPM、Key、LUFS、LRA、Energy、Danceability、Key Confidence、Drop LUFS、Analysis Version、Comment 和五个 Discogs JSON；标准 Genre 按现有阈值策略判断。

**2026-08-24 实际验收结果：** 最新 arm64 bundle 已运行三次 `libraryAnalysis --exercise-cancel-resume`，报告为 `/private/tmp/w4dj-headless-acceptance/library-analysis-16-final.jsonl`。每次均在首曲 `Hallelujah - Leonard Cohen.mp3` 的 `extractingMusiCnn` 接近 `17344` 帧时 WebContent 被系统重启，报告出现 3 个不同 `runId`，没有任何 `persisting` 事件；App 由 `SIGTERM` 正常停止。SQLite/JSON 保持基线 `completed=8 / failed=1 / notAnalyzed=7`，未写入用户音频。问题已缩小到隐藏 WebContent 的 Essentia/TensorFlow 高峰或 WebKit 进程重启，不能将 16 首、取消/恢复、SQLite/JSON/ExifTool 验收写成通过；下一步需单独修复该运行时稳定性后再重跑。

---

### Task 4: 后续验收全部迁移到后台场景

**Files:**

- Modify: `docs/superpowers/plans/2026-08-23-w4dj-incomplete-plans-master.md`
- Modify: `docs/superpowers/plans/2026-08-19-task13-external-acceptance.md`
- Modify: `docs/superpowers/plans/2026-08-23-analysis-resume-and-performance.md`
- Modify: `docs/superpowers/plans/2026-08-23-flac-cover-database-recovery.md`
- Modify: `docs/superpowers/plans/2026-08-23-manual-netease-metadata-database.md`
- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`

| 待验收范围 | 后台验收方式 | 不能自动化的边界 |
| --- | --- | --- |
| 16 首与后续整库分析 | hidden WebView + shared runner + SQLite/JSON/ExifTool | 无 |
| 89 首 FLAC/网易云数据库 | `flacCoverRecovery` 场景，数据库只读，输出写入独立验收目录 | 缺少真实数据库或素材时阻塞 |
| MP3/FLAC/WAV/AIFF | `externalFormats` 场景 + ffprobe/ExifTool | 无对应编解码器时阻塞 |
| Energy 十级 | jsdom/隐藏 WebView 检查 DOM 文本、ARIA、筛选和排序；校验原始 RMS² 不变 | 不再要求打开 Dashboard 人工 hover |
| Emotion | `emotionManifest` 后台生成与校验 manifest | 主观盲听必须由人完成，但不要求打开 W4DJ App GUI |
| Windows | Windows runner 执行同一 CLI 场景和文件回读 | 没有 Windows 环境时阻塞 |
| Rekordbox | 后台生成相对路径播放列表/XML并核对文件与标签 | Rekordbox 实际导入、波形和播放由外部人工验收 |
| DMG | `hdiutil attach -nobrowse`、签名/资源检查、挂载后二进制 `bundleSmoke` | 正式签名/公证权限缺失时阻塞 |

- [ ] 历史记录保持原样；在各计划增加“由本计划取代 GUI 触发”的明确说明，不回写过去未执行步骤为已完成。
- [ ] 所有未来 W4DJ 真实验收禁止以“后台可访问性按钮”作为触发条件。
- [ ] 外部人工项目必须与后台机器验收分栏报告，不能因无法自动化而阻塞彼此独立的后续任务。

---

### Task 5: 完整验证与交付报告

- [ ] 运行：

```bash
pnpm --dir app test -- --run
pnpm --dir app build
cargo test --all
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

- [ ] 构建 arm64 App，检查 `3.2.0-beta.3`、隐藏场景不创建可见窗口、正常启动仍显示原 GUI。
- [ ] 最终报告包含场景命令、退出码、JSONL 路径、逐曲结果、取消/恢复证据、SQLite/JSON/ExifTool 一致性、耗时、停滞检测、外部限制、`git status`、`git diff --stat` 和最新 App 链接。
- [ ] 不自动生成用户错误报告，不 commit、push、merge、release。

## Supersession

本计划自 2026-08-24 起取代所有尚未执行的“打开 W4DJ GUI、使用可访问性按钮、手动点击继续/重新分析、通过截图观察进度”验收步骤。既有历史验收记录不改写；只有后续未完成项改用后台场景。
