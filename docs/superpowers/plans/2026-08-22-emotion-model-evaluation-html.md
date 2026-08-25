# 情绪模型主观验收 HTML 工具实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `执行计划代理` or inline execution to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Do not commit or push until the user explicitly says “定稿”。

> **2026-08-24 验收入口更新：** manifest 导出、路径校验和数据统计使用 `2026-08-24-headless-acceptance.md` 的后台场景。主观盲听保留在独立 HTML 工具中，不启动或操作 W4DJ App GUI。

**Goal:** 在工作区提供一个独立 HTML 验收工具，用同一批 10 秒音频片段随机比较旧 Mood 基线、emoMusic、MuSe 和 MIREX，并导出可复核的主观选择报告。

**Architecture:** Rust/Tauri 只读导出 `emotion-evaluation-manifest.json`，从 W4DJ SQLite 选择可用输出并保留四套系统的原始预测；HTML 工具不重新运行模型，只负责选择音频目录、播放片段、匿名随机呈现、IndexedDB 断点保存和 JSON/CSV 导出。前端纯逻辑放在可测试的模块中，页面只调用这些模块。

**Tech Stack:** Rust/Tauri、rusqlite、serde_json；原生 HTML/CSS/ES modules、Vitest、IndexedDB、浏览器目录选择器。

## Global Constraints

- 验收工具放在 `tools/emotion-evaluation/`，不加入 Dashboard 或正式设置界面。
- 四个竞争系统固定为旧五 Mood 头基线、emoMusic、MuSe、MIREX；旧五头只算一张基线卡片。
- 每首歌使用同一个最高能量/Drop 优先的 10 秒片段；模型卡片顺序和歌曲顺序使用保存的随机种子随机化。
- HTML 读取 manifest，不在浏览器重复运行 Essentia/TensorFlow，不从网络下载模型。
- 音频通过用户选择的输出文件夹和 `relativePath` 匹配；缺失、不可播放、模型缺失或失败不计入胜率分母。
- 每首提交立即写 IndexedDB；验收结果不写入 `w4dj.sqlite3`、`track-analysis.json`、音频或转换历史。
- 不修改版本号，不创建 baseline/hash/frozen contract，不 commit、push、merge 或发布。

---

### Task 1: W4DJ manifest 导出

**Files:**
- Modify: `src/w4dj_library.rs`
- Modify: `src-tauri/src/main.rs`
- Add: `src-tauri/src/bin/export_emotion_evaluation_manifest.rs`
- Test: `tests/w4dj_library.rs`

**Interfaces:**
- `EmotionEvaluationManifest`：`schemaVersion/ sessionId/ seed/ sampleSize/ clipPolicy/ tracks`，使用 camelCase JSON。
- `W4djLibrary::emotion_evaluation_manifest(count: usize, seed: u64) -> W4djResult<EmotionEvaluationManifest>`：只读取 `status='available'` 的输出记录，按种子无放回抽样。
- `export_emotion_evaluation_manifest(output_path, count, seed)`：Tauri/CLI 共用导出函数，原子写 JSON，不触发分析或改库。

- [x] **Step 1: 为 manifest 和抽样写失败测试**

在 `tests/w4dj_library.rs` 构造三条 available 输出和一条 missing 输出，调用 manifest 构建函数，断言只选择 available、相同种子顺序相同、不同种子顺序可变，且每条包含 `relativePath`、10 秒片段字段和四套模型状态。

- [x] **Step 2: 实现只读查询和相对路径**

从 `w4dj_track_meta`、`tracks` 和 `analysis_results` 查询输出记录；相对路径以登记的 `output_root` 为根计算，无法裁剪时使用文件名并标记路径降级。解析 `analysis_json.highLevel` 中已有 `mood`，读取未来的 `emotionCandidates.emomusic`、`emotionCandidates.muse` 和 `moodCluster`；字段不存在时写 `missing`，不伪造预测。

- [x] **Step 3: 实现片段和随机种子**

优先使用已有 `dropAnalysis.segmentStartSeconds` 作为 Drop 窗口提示，并将起点限制到 `[0, duration-10]`；没有提示时用随包/系统 FFmpeg 将音频降为 8 kHz 单声道，按 1 秒步长寻找 10 秒 RMS 能量最高窗口；解码不可用才回退到起点并在 `clipSelection` 标记原因。短于 10 秒的歌曲使用整首并记录实际时长。使用固定 seed 的 Fisher-Yates 抽样，manifest 保存最终顺序和 seed。

- [x] **Step 4: 暴露导出入口并验证**

加入 `export_emotion_evaluation_manifest` Tauri 命令和 CLI 二进制参数 `--output <path> --count <n> --seed <u64>`；输出使用临时文件后 rename。运行 `cargo test --test w4dj_library` 和 `cargo test --manifest-path src-tauri/Cargo.toml export_emotion_evaluation_manifest`，确认旧数据库和空分析记录仍可导出。

### Task 2: 验收纯逻辑模块

**Files:**
- Create: `tools/emotion-evaluation/evaluator.js`
- Create: `tools/emotion-evaluation/evaluator.test.js`

**Interfaces:**
- `shuffleWithSeed<T>(items: T[], seed: number): T[]`
- `matchRelativeAudioFiles(files: readonly RelativeAudioFile[], relativePaths: readonly string[]): Map<string, RelativeAudioFile>`
- `scoreSelection(selection: EvaluationSelection): ScoreSummary`
- `exportEvaluationJson(session: EvaluationSession): string`
- `exportEvaluationCsv(session: EvaluationSession): string`

- [x] **Step 1: 写随机、路径和计分测试**

覆盖相同 seed 顺序稳定、不同 seed 顺序变化、目录分隔符归一化、同名不同目录不串配、唯一胜者得 1、两/三方并列分别得 0.5/1/3、都不符合不进分母、模型缺失不允许成为胜者。

- [x] **Step 2: 实现类型和纯函数**

定义 `ManifestTrack`、`ModelCard`、`HumanEmotionLabel`、`EvaluationSelection`、`EvaluationSession` 和 `ScoreSummary`；模型 ID 只允许 `legacyMood/emomusic/muse/mirex`。所有函数不得访问 DOM、IndexedDB 或文件系统。

- [x] **Step 3: 实现 JSON/CSV 导出**

JSON 保留 manifest、匿名卡片到真实模型的映射、主观标签、选择、随机顺序和计分中间值；CSV 每行一首歌，包含 trackId、clip、humanLabel、winner、validSample、四套状态和分数。

### Task 3: 独立 HTML 验收页面

**Files:**
- Create: `tools/emotion-evaluation/index.html`
- Create: `tools/emotion-evaluation/main.js`
- Create: `tools/emotion-evaluation/styles.css`
- Create: `tools/emotion-evaluation/README.md`
- Modify: `app/package.json` only if a workspace script is needed

**Interfaces:**
- 页面事件：`loadManifest`、`chooseAudioDirectory`、`submitHumanLabel`、`submitModelChoice`、`pauseSession`、`resumeSession`、`exportJson`、`exportCsv`。
- IndexedDB store：数据库名 `w4dj-emotion-evaluation`，store `sessions`，主键 `sessionId`。

- [x] **Step 1: 写页面最小流程测试**

使用 jsdom/DOM 测试 manifest 载入、无模型结果时卡片禁用、主观标签必须先提交、匿名 A/B/C/D 顺序显示、提交后进度递增、刷新后恢复当前 session。

- [x] **Step 2: 实现 manifest 和目录选择**

使用 `<input type=file accept='.json'>` 读取 manifest，使用 `webkitdirectory` 选择输出目录；把文件的 `webkitRelativePath` 归一化后交给 `matchRelativeAudioFiles`。找不到文件显示“文件缺失”，不阻断其它歌曲。

- [x] **Step 3: 实现两阶段答题**

第一阶段只显示标题、艺术家、10 秒 audio 控件和六个主观标签加“无法判断”；提交后才渲染匿名四卡片。卡片只显示规范化输出，不显示真实模型名；缺失/失败卡片不可选。选择并列或都不符合后写入 IndexedDB 并进入下一首。

- [x] **Step 4: 实现恢复和导出**

每次提交后保存 session；页面重新打开时列出未完成 session 并恢复 manifest、音频匹配、答题位置和已提交记录。导出按钮调用纯逻辑模块下载 JSON 和 CSV，失败时保留会话并显示重试。

### Task 4: 文档、构建和人工验收

**Files:**
- Modify: `docs/superpowers/specs/2026-08-22-emotion-model-evaluation-html-design.md` only for implementation notes
- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`

- [x] **Step 1: 增加本地运行说明**

README 写明：在仓库根目录运行 `python3 -m http.server 1431 --directory tools/emotion-evaluation` 提供静态文件；浏览器选择 manifest 和输出目录；完成后点击导出 JSON/CSV。该工具不依赖 app Vite，也不会加载模型。

- [x] **Step 2: 运行自动验证**

Run: `/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node app/node_modules/vitest/vitest.mjs run tools/emotion-evaluation/evaluator.test.js tools/emotion-evaluation/main.test.js`

Run: `/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node app/node_modules/vite/bin/vite.js build` (当前沙箱使用等价的临时 cache/outDir 配置)

Run: `cargo test --all`

Run: `cargo fmt --all -- --check`

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Run: `git diff --check`

Expected: 纯函数、Rust manifest、页面构建和现有回归全部通过。

- [ ] **Step 3: 执行分层人工验收**

先用 5 首夹具验证播放/提交/恢复/导出，再用 20 首真实歌曲验证 `relativePath` 和缺失处理，最后按 100 或 200 首运行完整批次；核对四套系统映射、有效样本分母、随机 seed 和 W4DJ 数据库未被修改。
