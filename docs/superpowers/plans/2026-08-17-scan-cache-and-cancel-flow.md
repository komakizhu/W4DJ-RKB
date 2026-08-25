# 扫描缓存与阶段取消/断点续传 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** 将扫描从转换流程中解耦，加入独立的 scan-cache.json 增量扫描缓存，把扫描、转换、增强分析明确显示为三个阶段，并让扫描、转换、增强分析分别在正确阶段提供可取消和可继续的行为。

**Architecture:** 扫描阶段先按任务独立收集输入文件并读写扫描缓存；缓存只复用未变化文件的扫描结果，输出目录、冲突策略和文件名规则变化时重新计算输出计划。扫描结束后，扫描后转换模式继续使用现有“转换前确认”页面；直接转换模式不弹确认窗口，基础检查通过后自动进入转换。转换成功后立即生成正式文件，增强模式的 Essentia 分析在后台补写元数据；分析取消或失败不会删除已经成功生成的文件。

**Tech Stack:** Rust/Tauri backend, walkdir/现有同步与预览模块, serde JSON cache, TypeScript frontend, Vitest, Vite production build, Cargo tests/fmt/clippy, Tauri Apple Silicon app bundle.

## Global Constraints

- 只在当前 3.2.0 工作分支上实现；不使用 3.0.0 源码，不合并其他分支，不修改版本号。
- 当前工作区可能存在用户未提交改动。实施前只检查并记录基线，不使用破坏性回退，不覆盖或删除无关未跟踪文件。
- 本计划不自动提交、推送 GitHub、创建 Release 或修改 Actions；完成后只展示测试结果、git status、diff 摘要和本地 Apple Silicon App 地址，等待用户确认。
- 普通模式不运行 Essentia；增强模式沿用现有分析、元数据、报告和 track-analysis.json 逻辑。本计划新增的是扫描缓存，不替换分析缓存。
- 不新增“扫描后新增曲目”复选框。用户所说的控件指现有“转换前确认”流程；保留当前确认页面及其统计信息，现有“已存在文件：跳过/覆盖/更新元数据”继续作为冲突策略来源。
- 扫描、转换和增强分析均按歌曲级别取消和继续，不做单个音频文件内部的字节级断点续传。当前歌曲取消时从头重做。
- 临时文件只清理当前批次中未完成的临时结果；已经成功写入正式输出目录的文件保留。失败、取消和待继续文件写入转换历史及错误报告。

---

## Task 1: 建立独立扫描缓存模型和原子文件操作

**Files:**

- Create: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/scan_cache.rs
- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/main.rs
- Test: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/scan_cache.rs unit tests or /Users/mac2/Documents/W4DJ RKB/src-tauri/tests/scan_cache.rs

**Interfaces:**

在 Rust 中建立独立于分析缓存的缓存类型，至少包含以下数据：

~~~rust
pub const SCAN_CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanCacheEntry {
    pub source_path: String,
    pub source_root: String,
    pub output_directory: String,
    pub filename_rule: String,
    pub netease_filename_format: String,
    pub size_bytes: u64,
    pub modified_at_ms: Option<u64>,
    pub derived_name: String,
    pub scan_issue: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanCache {
    pub schema_version: u32,
    pub entries: BTreeMap<String, ScanCacheEntry>,
}

pub fn load_scan_cache(path: &Path) -> io::Result<ScanCache>;
pub fn save_scan_cache_atomic(path: &Path, cache: &ScanCache) -> io::Result<()>;
pub fn clear_scan_cache(path: &Path) -> io::Result<()>;
pub fn can_reuse_entry(
    entry: &ScanCacheEntry,
    source_path: &Path,
    source_root: &Path,
    output_directory: &Path,
    filename_rule: &str,
    netease_filename_format: &str,
    size_bytes: u64,
    modified_at_ms: Option<u64>,
) -> bool;
~~~

- 将缓存文件放在现有应用数据目录，文件名固定为 scan-cache.json。
- 使用临时文件写入、flush 后 rename 的原子保存方式；不得直接覆盖正式 JSON。
- 文件不存在、版本号不兼容或 JSON 损坏时，返回空缓存并让本次扫描全量执行；同时把缓存损坏写入调试报告，不静默丢失诊断信息。
- can_reuse_entry 必须同时比较规范化源文件路径、源目录、文件大小、修改时间、输出目录、文件名规则和网易云文件名规则。
- 缓存只保存扫描结果和扫描问题，不把目标文件存在与否永久当作真相；每次扫描仍重新检查输出目录，避免输出被用户删除后被缓存错误跳过。
- 清除缓存只删除 scan-cache.json 并刷新内存缓存，不删除 track-analysis.json、Essentia 模型或输出音频。

**Tests:**

- 覆盖首次加载、保存后重新加载、原子保存失败、损坏 JSON、旧 schema 版本和清除缓存。
- 覆盖同一文件大小或修改时间变化、输出目录变化、命名规则变化时不能命中。
- 覆盖源文件未变化但正式输出被删除时，缓存可以复用源扫描信息，同时重新生成待转换输出计划。

## Task 2: 让扫描器按任务增量复用缓存并保存独立进度

**Files:**

- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/sync.rs
- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/preview.rs
- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/main.rs
- Test: /Users/mac2/Documents/W4DJ RKB/src-tauri/tests/preview.rs
- Test: /Users/mac2/Documents/W4DJ RKB/src-tauri/tests/sync_policy.rs

**Interfaces:**

新增任务级扫描进度，避免现在一个全局计数覆盖任务 1 和任务 2：

~~~rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanTaskProgress {
    pub task_index: usize,
    pub phase: ScanPhase,
    pub completed: usize,
    pub total: usize,
    pub current_file: Option<String>,
    pub reused: usize,
    pub rescanned: usize,
    pub canceled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanBatchProgress {
    pub conversion_mode: ConversionMode,
    pub phase: ScanPhase,
    pub tasks: Vec<ScanTaskProgress>,
    pub cancel_requested: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScanPhase {
    Scanning,
    Converting,
    Analyzing,
    Completed,
    Canceled,
    Failed,
}
~~~

- 保留现有扫描命令名称，扩展其返回结构，避免前端和旧测试无谓地切换接口。
- run_scan_task 为任务 1、任务 2 分别加载同一个扫描缓存，但按规范化源路径保存各自条目和进度。
- 对每个候选音乐文件先读取大小与修改时间，再判断缓存；命中时复用文件类型、解析出的歌曲名、扫描问题等结果，不重复执行昂贵的元数据/网易云文件名解析。
- 新增文件、大小变化、修改时间变化、命名规则变化或输出目录变化时重新扫描，并在该文件完成后立即更新内存缓存。
- 扫描完成后重新检查输出目录，重新计算“新增/已存在/将跳过/错误”等预览计划；不得直接复用旧的 destination 状态。
- 输入文件已删除时从当前任务结果移除对应缓存条目；未访问的文件不能标记为已完成。
- 取消扫描在当前文件边界响应：已完成条目写入缓存，未访问文件保持未完成状态；取消后不进入转换，也不显示转换确认页。
- 直接转换与扫描后转换使用同一套扫描缓存和扫描进度，区别只在扫描完成后的 UI 分支。

**Tests:**

- 同一目录第二次扫描时，未变化文件进入 reused，新增/变化文件进入 rescanned。
- 删除输入文件、删除输出文件、改变输出目录和改变命名规则时，输出计划与计数正确。
- 任务 1 扫描完成而任务 2 仍在扫描时，两栏进度互不覆盖。
- 扫描过程中取消，缓存保留已完成项，下一次扫描从未完成/新增/变化文件继续。
- 直接模式和扫描后转换模式都能调用同一取消扫描命令。

## Task 3: 重构后端阶段状态、取消边界和断点继续

**Files:**

- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/main.rs
- Modify if required by existing history contract: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/history.rs
- Modify if required by existing temp-file cleanup: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/sync.rs
- Test: /Users/mac2/Documents/W4DJ RKB/src-tauri/tests/

**Implementation:**

- 统一后台批次状态为扫描、转换、增强分析、完成、取消、失败；每个任务保留自己的阶段、完成数、总数、当前文件和错误。
- 扫描后转换模式：扫描完成后只打开现有“转换前确认”页面；用户确认后进入转换。
- 直接转换模式：扫描完成后不打开任何扫描窗口或确认窗口；基础检查通过后直接转换。
- 扫描阶段两种模式都可取消。扫描取消不启动转换，保留已经完成的扫描缓存。
- 转换开始后，两种模式都可取消。取消当前文件时删除其临时文件，不删除已成功生成的正式文件；未开始文件写入待继续列表。
- 增强分析阶段两种模式都可取消。已完成歌曲的分析缓存和元数据更新保留；当前未完成歌曲不写入不完整结果；已转换音频不删除。
- 错误发生时终止当前批次，删除当前批次的未完成临时文件，保留成功正式文件，写入完整错误报告和“部分完成”历史；不自动重试。
- 继续逻辑按歌曲执行：优先读取现有历史中的 pending_files，已完成文件不重复转换；被取消的当前歌曲下次从头转换；扫描缓存和分析缓存分别决定扫描/分析是否重做。
- 当有待继续文件时，后端状态提供“继续转换”语义；没有待继续文件时显示“同时开始”。不引入音频中间位置断点。
- 直接模式不显示暂停按钮，但保留全批次“取消转换”；不改变已有单曲删除、清空和历史记录语义。

**Tests:**

- 扫描阶段取消不会调用转换；转换阶段取消只保留已完成文件；分析阶段取消不删除已转换文件。
- 当前文件临时文件在取消/失败后被清理，正式文件和待继续列表保持正确。
- 重新点击继续后只处理未完成歌曲，当前被中断歌曲从头开始。
- 普通模式不执行增强分析；增强模式转换完成后才进入分析阶段。

## Task 4: 前端移除扫描弹窗并把阶段进度并入任务卡

**Files:**

- Modify: /Users/mac2/Documents/W4DJ RKB/app/src/app.ts
- Modify: /Users/mac2/Documents/W4DJ RKB/app/src/styles.css
- Test: /Users/mac2/Documents/W4DJ RKB/app/src/app.test.ts

**Implementation:**

- 删除主渲染流程对 renderScanModal(scanProgress) 的依赖；扫描状态不再通过遮罩弹窗展示。
- 在任务 1、任务 2 的现有进度条区域显示对应 ScanTaskProgress：
  - 扫描中：显示扫描完成数/总数、当前文件名和“扫描中”。
  - 转换中：显示现有转换进度和“转换中”。
  - 分析中：显示增强分析进度和“分析中”。
  - 完成：显示完成状态。
- 保持两栏布局和现有进度条尺寸，避免因为阶段文字变化造成任务卡跳动；长路径和当前文件名使用已有截断规则。
- 全局按钮行为固定如下：

| 阶段 | 扫描后转换 | 直接转换 | 可取消 |
| --- | --- | --- | --- |
| 扫描中 | “取消扫描” | “取消扫描” | 两者都可取消 |
| 转换前确认 | 显示现有“转换前确认”页面 | 不显示 | 确认前可关闭，不会开始转换 |
| 转换中 | “取消转换” | “取消转换” | 两者都可取消 |
| 增强分析中 | “取消分析” | “取消分析” | 两者都可取消 |
| 完成 | “同时开始” | “同时开始” | 不可取消 |
| 出错/部分完成 | “继续转换”或“同时开始” | “继续转换”或“同时开始” | 当前操作已结束 |

- 取消按钮只发送对应 action：cancel-scan、cancel-conversion、cancel-analysis，不通过修改普通模式或清空任务来模拟取消。
- 扫描后转换保留现有“转换前确认”统计、确认、取消和返回编辑流程；不新增“扫描后新增曲目”复选框。
- 直接转换无弹窗、无确认页、无暂停按钮；发生错误时在任务卡和错误报告显示原因。
- 保留现有 WAV/AIFF、普通/增强、兼容/无损动画和防闪烁逻辑，不因阶段状态重建这些按钮 DOM。

**Frontend interfaces:**

- 将现有 AppScanProgress 拆为任务级结构，和 Rust ScanBatchProgress 对齐。
- startScan、loadScanState、cancelScan 的服务类型使用新结构；对旧测试保留必要的默认字段适配。
- 增加前端状态机，禁止旧的扫描状态回调覆盖更新后的转换/分析阶段。

**Tests:**

- 扫描后转换和直接转换都不渲染扫描弹窗。
- 两个任务的扫描进度分别显示，任务 1 的更新不会改变任务 2。
- 扫描中按钮显示且触发“取消扫描”；转换中触发“取消转换”；增强分析中触发“取消分析”。
- 扫描后转换只在扫描完成后显示现有“转换前确认”页面；直接转换从扫描直接进入转换。
- 阶段切换不重建模式切换、WAV/AIFF 和高级选项 DOM。

## Task 5: 增加扫描缓存清除入口并调整增强缓存入口

**Files:**

- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/main.rs
- Modify: /Users/mac2/Documents/W4DJ RKB/app/src/app.ts
- Modify: /Users/mac2/Documents/W4DJ RKB/app/src/styles.css
- Test: /Users/mac2/Documents/W4DJ RKB/app/src/app.test.ts

**Implementation:**

- 增加 clear-scan-cache Tauri command，清除 scan-cache.json 并刷新当前内存中的扫描缓存。
- 保留 clear-analysis-cache 后端命令，但前端显示名称统一改为“清除增强模式缓存”。
- 在高级选项中把“清除增强模式缓存”放到“下载分析模型”右侧；“清除扫描缓存”作为独立按钮放在同一组设置中，颜色和现有按钮样式对齐且保证可见、可点击。
- 清除扫描缓存不删除分析缓存、Essentia 模型或转换历史；清除增强模式缓存不删除扫描缓存。
- 操作成功后刷新按钮状态并在报告/状态提示中说明“下次扫描将全量扫描”或“下次增强模式将重新分析”。

**Tests:**

- 两个按钮都能显示、点击和调用正确 command。
- 清除扫描缓存后下一次扫描不命中任何旧扫描项；分析缓存和模型仍存在。
- 清除增强模式缓存后扫描缓存仍命中；增强分析重新执行。
- 中英文 UI 文案、按钮禁用态和错误提示正确。

## Task 6: 补齐报告、历史和异常场景可观察性

**Files:**

- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/main.rs
- Modify: /Users/mac2/Documents/W4DJ RKB/src-tauri/src/history.rs
- Modify: /Users/mac2/Documents/W4DJ RKB/app/src/app.ts
- Test: /Users/mac2/Documents/W4DJ RKB/src-tauri/tests/
- Test: /Users/mac2/Documents/W4DJ RKB/app/src/app.test.ts

**Implementation:**

- 转换报告中记录每个任务的扫描阶段、缓存命中/重新扫描数量、取消位置、转换完成数、分析完成数和失败原因。
- 历史记录明确标记“完成”“部分完成”“已取消”“失败”，保存已完成和待继续文件列表。
- 扫描缓存损坏、目录无权限、输入文件消失、输出目录不可写、磁盘空间不足和增强分析失败都要有用户可见提示及完整错误报告。
- 日志中区分扫描取消、转换取消和分析取消，避免用户只看到统一的“操作失败”。
- 不把“模型未下载”视为转换失败；增强模式完成基础转换后记录“增强模型未启用/分析跳过”。

**Tests:**

- 模拟扫描缓存损坏、输入目录无权限、输出目录无权限和文件在扫描中被删除。
- 模拟转换失败、分析失败和用户取消，确认报告与历史不互相覆盖。
- 重新打开软件后仍能加载阶段状态、待继续历史和扫描缓存；不会把取消批次误报为完成。

## Task 7: 完成验证并构建本地 Apple Silicon App

**Files:**

- No source changes beyond Tasks 1–6.
- Build output: /Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app

**Verification commands:**

在当前分支完成实现后依次运行：

~~~bash
cd "/Users/mac2/Documents/W4DJ RKB/app" && pnpm test
cd "/Users/mac2/Documents/W4DJ RKB/app" && pnpm build
cd "/Users/mac2/Documents/W4DJ RKB" && cargo test --workspace
cd "/Users/mac2/Documents/W4DJ RKB" && cargo fmt --all -- --check
cd "/Users/mac2/Documents/W4DJ RKB" && cargo clippy --workspace --all-targets --all-features -- -D warnings
cd "/Users/mac2/Documents/W4DJ RKB" && cargo tauri build --target aarch64-apple-darwin --bundles app
~~~

- 如果仓库当前脚本使用不同的测试入口，只允许使用仓库已有脚本对应的等价命令，不改变构建配置。
- 使用临时测试目录验证两任务、少量文件、大量文件、缓存命中、缓存清除、扫描取消、转换取消、分析取消、断点继续和错误清理。
- 手动确认：扫描进度只出现在任务卡；直接转换无弹窗；扫描后转换有且只有现有“转换前确认”页面；三个阶段按钮文案准确；完成/部分完成/失败状态准确。
- 检查 Apple Silicon App 可打开，并把绝对路径或可点击本地链接交付给用户。
- 最终只展示 git status --short、git diff --stat、测试摘要和 App 地址，不执行 commit、push 或 release。

## Acceptance Checklist

- [ ] scan-cache.json 独立存在，能按路径、大小、修改时间、输出目录和命名规则增量复用。
- [ ] 两个任务拥有独立扫描进度，扫描不再弹出“扫描歌曲”窗口。
- [ ] 扫描后转换保留现有“转换前确认”页面，不新增重复复选框。
- [ ] 直接转换无弹窗，扫描完成后自动转换。
- [ ] 扫描中两种模式都显示“取消扫描”。
- [ ] 转换中两种模式都显示“取消转换”。
- [ ] 增强分析中两种模式都显示“取消分析”。
- [ ] 取消按歌曲边界生效，正式完成文件保留，当前临时文件清理。
- [ ] 继续操作按歌曲恢复，不做音频内部断点；被中断歌曲从头开始。
- [ ] “清除扫描缓存”和“清除增强模式缓存”分开，互不删除对方数据。
- [ ] 报告与历史可以区分扫描取消、转换取消、分析取消、失败和部分完成。
- [ ] 前端、Rust、格式、Clippy 和 Apple Silicon App 验证完成。

## Execution Order

- [ ] 实施前检查当前分支、工作区状态和现有测试基线。
- [ ] 按 Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6 顺序实施；每个任务完成后先运行对应测试。
- [ ] 完成 Task 7 的全量验证和本地 App 构建。
- [ ] 向用户展示状态、diff 摘要、验证结果和本地 App 地址，等待“定稿”后再考虑提交或推送。

## 2026-08-25 增量验收：取消扫描与动态总数

本次在既有实现上完成扫描取消与进度修复：输入/输出目录先做可取消的轻量预枚举，正式扫描与网易云 locator 匹配逐条检查取消标记；任务卡按阶段显示 `processed/total`，前端高频轮询只做局部 DOM 更新。`cancel_scan` 立即发布 `cancelling`，后台以 `cancelled` 终态收口并保留已有缓存。

真实 T7 输入目录验收为 1088/1088 候选、1088/1088 元数据事件、3.52 秒。前端 Vitest 194/194、根 `cargo test --all`、Tauri 58/58、TypeScript、Vite、Tauri check、Tauri 严格 Clippy、fmt 和 diff-check 通过；根 all-targets 严格 Clippy 仍有工作树既有 legacy dead_code 与 `map_identity`。arm64 App 已构建于 `/Users/mac2/Documents/W4DJ RKB/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`（3.2.0-beta.3）。GUI 实际点击取消、Windows/Rekordbox 和未挂载外置卷仍待现场验收。
