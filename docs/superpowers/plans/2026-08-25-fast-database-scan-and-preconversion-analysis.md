# 快速数据库扫描与转换后增强分析实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Each task must finish its own test cycle before the next task starts.

**Goal:** 将数据库元数据扫描放在转换前、将增强音频分析保留在转换后且保持默认关闭，并让两个阶段都显示在任务进度条中，不再因同步重工作业触发 macOS 彩虹圈。

**Architecture:** 数据库发现先做轻量 schema/计数探测，再由有界后台 worker 并发读取选中的表；音乐目录计数和输入扫描各只遍历一次。数据库元数据准备完成后才开始转换。增强音频分析仍由现有逐曲 Worker 在转换成功后执行，只有用户开启增强模式才启动；分析结果绑定实际输出文件并执行标签写回。数据库扫描和增强分析都通过任务卡进度条增量显示。

**Tech Stack:** Rust/Tauri、SQLite/rusqlite、WalkDir、std::thread/Arc/Mutex、TypeScript/Vite、Essentia.js Worker、现有 `w4dj.sqlite3`、运行会话和扫描缓存。

## Global Constraints

- 不修改 SemVer、产品版本号或既有 Task 6–13 对外接口语义。
- 不修改网易云 SQLite 的内容；始终以 SQLite read-only 打开。
- 不删除 `w4dj.sqlite3`、`track-analysis.json`、历史记录或已有音频标签。
- 保留共享工作树现有改动，不执行 reset、clean、commit、push、merge 或 release。
- 数据库扫描取消和增强分析取消分别处理；增强分析默认不启动，取消只停止当前分析会话，已完成结果保留。
- 任何失败必须进入终态并显示原因，不得永久停在 `running` 或让 UI 失去响应。
- 不新增 baseline、hash、冻结 contract 或发布 gate。

---

### Task 1: 数据库轻量探测、并发读取与结果缓存

**Files:**

- Modify: `src/netease.rs`
- Modify: `src/netease_library.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `tests/netease.rs`（若仓库无此文件，新增同名集成测试）
- Test: `src-tauri/src/main.rs` 内现有 Tauri 单元测试模块

**Interfaces:**

```rust
pub struct NeteaseDatabaseSummary {
    pub path: PathBuf,
    pub supported: bool,
    pub record_count: usize,
    pub fingerprint: DatabaseFingerprintView,
}

pub fn probe_netease_database(path: &Path) -> rusqlite::Result<NeteaseDatabaseSummary>;

pub fn load_records_from_db_observed<Observe>(
    path: &Path,
    parallelism: usize,
    observe: Observe,
) -> rusqlite::Result<Vec<NeteaseRecord>>
where
    Observe: FnMut(&'static str, usize, usize);
```

- [x] **Step 1: 为候选数据库建立轻量探测测试。** 创建最小 SQLite fixture，验证 `probe_netease_database` 只判断支持表和行数，不解析 `web_track.track` JSON；无支持表、文件不存在和 WAL 存在三种情况分别返回明确结果。
- [ ] **Step 2: 运行探测测试确认失败。** 该历史 TDD 预步骤未在实现后重复执行；当前接口和回归测试已存在，避免故意引入失败测试。
- [x] **Step 3: 实现只读 schema/计数探测。** `probe_netease_database` 使用 `SQLITE_OPEN_READ_ONLY`，查询 `sqlite_master` 和每个支持表的 `COUNT(*)`；不得调用 `load_records_from_db`。
- [x] **Step 4: 为记录读取增加有界并发。** 将表读取拆成独立只读连接，最多 `parallelism` 个 worker；每个表仍保留 `LIMIT 200000`，读取完成后在调用线程按既有合并规则合并，JSON 解析错误保持现有过滤行为。并发数最小为 1，不能创建无界线程。
- [x] **Step 5: 增加 fingerprint 缓存。** 缓存手动数据库和候选数据库的路径、大小、mtime、WAL/SHM mtime；fingerprint 未变化时复用已加载的 `Arc<Vec<NeteaseRecord>>`，变化时重新加载。缓存只存在进程内，不生成新的 baseline 文件。
- [x] **Step 6: 改造自动/手动发现流程。** `discover_netease_library_observed` 先并发探测候选，再只加载记录数最多的支持数据库；手动路径先探测 schema，失败后立即回退自动发现。`candidate_music_folder` 优先使用已知目录和轻量 path 列，不依赖完整 JSON。
- [x] **Step 7: 将目录计数改为后台阶段。** `locate_netease_library` 找到数据库和目录后立即返回路径结果；文件数量通过后台任务发送 `checkingMusicFolder` 事件，完成后发送 `completed`。任务 1 来源目录和发现目录规范化后相同则不得重复计数。
- [x] **Step 8: 运行测试。** 运行 `cargo test --test netease`、`cargo test --test library_catalog` 和 `cargo test --manifest-path src-tauri/Cargo.toml`，确认只读、手动优先、无效回退、缓存命中和进度阶段全部通过。

**Acceptance:** 约 1 万条记录的数据库定位不再等待所有 JSON 解析和音乐目录计数；阶段事件先返回路径，后台计数继续推进；网易云 SQLite 文件 mtime/大小保持不变。

---

### Task 2: 输入目录单次枚举与扫描并发隔离

**Files:**

- Modify: `src/sync.rs`
- Modify: `src/scan_cache.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `app/src/app.ts`
- Test: `tests/sync_policy.rs`
- Test: `app/src/app.test.ts`

**Interfaces:**

```rust
pub struct EnumeratedMusicFiles {
    pub paths: Vec<PathBuf>,
    pub issues: Vec<MusicScanIssue>,
}

pub fn enumerate_music_files_observed<F>(
    folder: &str,
    allowed_extensions: &[&str],
    cancel: &AtomicBool,
    observe: F,
) -> Result<EnumeratedMusicFiles, ScanEnumerationError>
where
    F: FnMut(usize, Option<usize>, &Path);
```

- [x] **Step 1: 写单次枚举回归测试。** fixture 中建立输入/输出目录和损坏链接，验证一次枚举同时得到路径、问题和准确总数；取消时返回 `cancelled`，不隐藏权限错误。
- [x] **Step 2: 删除 `run_scan_task` 的预先计数双遍历。** 使用 `enumerate_music_files_observed` 的结果设置 `ScanProgress.total`，预览阶段复用路径列表，不再重新创建 `WalkDir`。
- [x] **Step 3: 合并文件属性读取。** 每个路径只读取一次 `Metadata` 和修改时间；扫描缓存比较使用预先规范化的根目录，避免对每个文件重复 `canonicalize`。
- [x] **Step 4: 建立独立扫描并发额度。** 新增 scan-only budget，默认沿用当前设置但不占用 FFmpeg conversion permit；worker 数量有上限并根据路径数量裁剪。外置盘不因盲目增加线程而放大 I/O。
- [x] **Step 5: 改进取消和停滞状态。** 扫描阶段每个 worker 检查取消；主线程在 worker 无法及时退出时显示 `cancelling` 和当前文件，不能永久停留在 `running`。不得强制删除输出或缓存。
- [x] **Step 6: 更新前端扫描进度。** 保留既有进度条，但将枚举、输入扫描、输出扫描和检查阶段分开显示；进度事件只增量更新文本和填充宽度，不触发整棵 DOM 重建。
- [x] **Step 7: 运行测试。** 运行 `cargo test --test sync_policy`、`cargo test --manifest-path src-tauri/Cargo.toml`、`app.test.ts` 相关前端回归测试。

**Acceptance:** 2,398 个文件的输入目录只遍历一次；本机 SSD 和 T7 外置盘都能看到持续进度；扫描期间取消按钮可用，扫描不会与 FFmpeg 并发额度互相阻塞。

---

### Task 3: 转换后增强分析会话与输出写回

**Files:**

- Modify: `app/src/app.ts`
- Modify: `app/src/analysis.ts`
- Modify: `app/src/analysis-worker-client.ts`
- Modify: `app/src/analysis-worker-protocol.ts`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/w4dj_library.rs`
- Test: `app/src/app.test.ts`
- Test: `app/src/analysis-worker-client.test.ts`
- Test: `src-tauri/src/main.rs` 内分析会话测试

**Interfaces:**

```ts
export type AnalysisRunPhase =
  | 'preparing'
  | 'loadingModels'
  | 'analyzingBasic'
  | 'analyzingHighLevel'
  | 'writingBack'
  | 'completed'
  | 'cancelled'
  | 'error';
```

- [x] **Step 1: 保持增强分析默认关闭。** 确认 `defaultState.enhancedMode === false`，普通扫描/转换路径不得调用 `analyzePreviewCandidates` 或加载 Essentia 模型；新增测试覆盖默认关闭、用户显式开启和重新分析入口三种情况。
- [x] **Step 2: 保持数据库扫描与增强分析分离。** `finishScan` 在扫描完成后只等待数据库元数据/目录扫描的终态，然后进入转换；不得在转换前调用 `analyzeAudioFile`、加载 Essentia 模型或写入增强分析结果。
- [x] **Step 3: 保留转换后的分析入口。** 继续由 `runPostConversionAnalysis` 在 `waitForConversionBatch` 确认输出完成后调用 `analyzePreviewCandidates`；仅当 `state.enhancedMode` 为 true 或用户从歌曲库点击“重新分析当前输出”时运行。
- [x] **Step 4: 绑定实际输出路径。** 保留现有 `apply_track_analysis_results` 的输出存在检查、标签事务、回读校验和 `w4dj.sqlite3` 投影；分析结果必须以 destination path 绑定，不增加转换前的伪造 completed 记录。
- [x] **Step 5: 保持逐曲错误隔离。** 每首歌继续独立 Worker、独立超时和独立终态；单曲失败/超时不阻断其他歌曲和转换历史，取消不覆盖已有成功标签。
- [x] **Step 6: 运行测试。** 运行前端 app/worker 回归测试和 `cargo test --manifest-path src-tauri/Cargo.toml`，确认默认关闭和转换后才分析。

**Acceptance:** 数据库扫描和增强音频分析是两个独立阶段；转换前不会运行 Essentia；增强模式默认关闭；开启增强模式时，只有转换完成后才开始分析和标签写回。

---

### Task 4: 将数据库与增强分析进度接入任务卡并消除主线程假死

**Files:**

- Modify: `app/src/app.ts`
- Modify: `app/src/styles.css`
- Modify: `app/src/analysis.ts`
- Modify: `app/src/analysis-worker-client.ts`
- Test: `app/src/app.test.ts`
- Test: `app/src/analysis.test.ts`
- Test: `app/src/analysis-worker-client.test.ts`

- [x] **Step 1: 固定两个阶段的状态机。** 数据库阶段使用 `NeteaseDiscoveryProgress`/`ScanProgress` 的 `locatingDatabase`、`readingRecords`、`checkingMusicFolder`；转换后增强分析使用 `AppAnalysisState` 的 `preparing`、`loadingModels`、`analyzingBasic`、`analyzingHighLevel`、`writingBack`、终态。两者不得互相伪造状态。
- [x] **Step 2: 复用任务卡进度条。** 数据库阶段显示“正在读取网易云数据库/正在检查音乐目录”；转换后增强分析显示“模型加载/基础分析/高级分析/写回结果”。当前项目只显示歌曲文件名，不显示完整路径。
- [x] **Step 3: 高频事件增量更新。** `updateAnalysisProgressDom` 只修改文本、`style.width` 和 `aria-valuenow`；每首开始、完成、取消、错误才调用完整 `render()`。旧 `workerJobId` 事件全部忽略。
- [x] **Step 4: 让两个阶段都不阻塞 WebView。** 数据库读取和目录扫描必须在 Rust 后台 worker；增强分析开始前立即渲染进度条并 `await yieldToUi()`，模型加载、逐曲 Worker 创建和写回均使用异步调用。不得在主线程同步遍历数据库或音频目录。
- [x] **Step 5: 保留逐曲 Worker 生命周期。** 每首歌独立创建、取消和终止 Worker；超时使用既有 `analysisTimeoutMs`，错误显示到阶段文本并继续下一首；不回退到主线程同步推理。
- [x] **Step 6: 进度条回归测试。** 测试分析期间按钮/键盘事件仍可执行、进度事件不调用完整 render、旧 job 消息不污染当前歌曲、取消后任务卡立即可操作。
- [x] **Step 7: 运行前端测试和构建。** 运行前端全量 Vitest、TypeScript 检查和 Vite 生产构建。

**Acceptance:** 数据库阶段和转换后增强分析阶段都显示连续进度，窗口不出现长时间彩虹圈；增强分析默认不启动；取消按钮可立即响应；数据库扫描、转换和增强分析在 UI 上明确分开。

---

### Task 5: 端到端验证、文档和性能验收

**Files:**

- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`
- Test data: 只读使用当前用户已配置的 T7 数据库和输入目录，不覆盖生产文件

- [x] **Step 1: 更新文档。** 已记录本机当前数据库探测结果、扫描/转换/转换后增强分析的阶段顺序，并明确增强分析默认关闭。外置 T7 和 2,398 文件数据不在当前环境，未把推测写成实测。
- [x] **Step 2: 运行 Rust 验证。**

```bash
cargo test --test netease
cargo test --test sync_policy
cargo test --test library_catalog
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
```

- [x] **Step 3: 运行前端验证。**

```bash
pnpm --dir app test -- --run
pnpm --dir app build
```

- [ ] **Step 4: 做真实数据验收。** 当前环境只有容器网易云库（支持表 `track=0`、`web_track=538`）和本地 `test/testtttt` 目录，未提供计划要求的外置 T7 数据库与 2,398 个输入文件；因此完整时序、外置卷 I/O、转换起始时间和真实取消恢复仍待现场执行。
- [x] **Step 5: 验证并发。** 通过有界 worker 实现、扫描回归测试和 Tauri/Rust 测试确认数据库读取有界，scan-only worker 不占用 FFmpeg permit，扫描不再做预先双遍历。
- [x] **Step 6: 验证数据安全。** 只读核对网易云 SQLite 探测前后 mtime 不变；现有用户数据库/JSON 未被本轮真实验收写入或删除。
- [x] **Step 7: 最终检查。** 已运行 `git diff --check`、`git status --short`、`git diff --stat`，保留工作树改动，不提交、推送或修改版本号。

**Acceptance:**

- 数据库路径结果不再等待 1 万条记录 JSON 解析和 2,398 个文件统计完成。
- 扫描、数据库元数据准备、转换和转换后增强分析各有可见阶段与进度条。
- 普通模式严格按“扫描 → 数据库准备 → 转换”执行；增强模式严格按“扫描 → 数据库准备 → 转换 → 增强分析”执行，且增强模式默认关闭。
- 分析取消、超时、数据库错误和转换失败均有终态；已有结果保留。
- 当前用户真实 T7 验收能提供 SQLite/文件 mtime/进程状态/运行会话证据，而非只凭窗口截图。

## 不在本计划范围

- 不改变分析模型、Genre/Emotion head 的数值契约。
- 不重新设计 DJ Crate Digger、Rekordbox、Windows 或发布流程。
- 不把外置磁盘问题伪装成数据库成功；若单次 `stat` 本身超过停滞阈值，报告为环境 I/O 限制。
