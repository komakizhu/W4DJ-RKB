# Discogs-EffNet DJ 元数据模型族接入实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `执行计划代理` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Do not dispatch subagents from a side conversation.

> **2026-08-24 验收入口更新：** 真实输出重分析、五个 head 状态和 ExifTool 回读全部改用 `2026-08-24-headless-acceptance.md` 的后台场景；不得打开 W4DJ App GUI。

**Goal:** 在现有 MusiCNN Style、基础 BPM/Key/Energy/Danceability 和 Worker 分析链之上，接入共享 Discogs-EffNet embedding 的 Mood/Theme、Approachability、Instrumentation、Timbre 和 Discogs Danceability 五个模型头，并把结果安全写入 W4DJ SQLite、兼容 JSON、Dashboard 和可选音频元数据。

**当前执行边界（2026-08-23）：** 官方 Discogs EffNet 共享 embedding、`genre_discogs400` head 以及本文五个额外 head 已完成离线转换、严格校验、内置资源、Worker 推理、独立投影、Dashboard 详情/筛选、报告和命名空间元数据接入。真实用户输出、ExifTool 回读和跨平台人工验收仍需在具备对应素材/环境时执行，不能用自动化测试替代。

**Architecture:** Discogs-EffNet 作为独立的 `embeddingFamily`，不复用 MusiCNN 的 `[N, 200]` 分类输入，也不覆盖现有 `style`、原始 Danceability 或网易云字段。一首歌在 Worker 中只提取一次 Discogs-EffNet 1280 维 embedding，再顺序运行五个轻量 head；每个 head 有独立状态、错误和结果，单个 head 缺失或失败不得清空其他 head。Dashboard 默认不增加拥挤的固定列，详情、筛选和可选列读取 W4DJ SQLite 的结构化投影。

**Tech Stack:** TypeScript/Vite/Vitest、Essentia.js WASM、TensorFlow.js GraphModel、Rust/Tauri、Serde、rusqlite、ExifTool/ffprobe。

## Global Constraints

- 不修改 SemVer、产品版本或现有 Task 6–13 接口含义。
- Discogs-EffNet 只在 Worker 中同步推理；主线程继续负责解码、重采样和 PCM 转移。
- 运行时不访问 `essentia.upf.edu` 或其它 CDN；所有模型资源必须通过离线转换、暂存重读和严格校验后随 App 安装。
- 共享一个 `discogs-effnet-bs64-1` embedding；不得为每个 head 重复加载或重复提取音频 embedding。
- 现有 MusiCNN `style`、`mood`、`instrument`、`danceability` 和网易云 Genre 保持原值；Discogs 结果使用独立字段。
- 当前 Danceability 的原始 Essentia 值和十级 S 曲线继续作为主值；Discogs Danceability 只能作为第二来源和交叉比较，不能覆盖 `W4DJ-Danceability`。
- 多标签 head 使用概率阈值筛选但保留原始概率；多分类 head 使用跨帧平均概率的最高类别；所有阈值、聚合方式、模型版本和缺失原因进入分析 JSON。
- 模型缺失、推理失败、取消或超时不得覆盖已有成功结果；单个 head 失败不得使基础分析或其它 head 失败。
- 不把 Approachability 当作质量、流行度或商业价值评分；显示时使用“可接近度/听众接受门槛”语义。
- 新增结果先写 W4DJ SQLite 和兼容 `track-analysis.json`；音频写回只增加命名空间明确的 W4DJ TXXX/Comment 字段并通过回读校验，不覆盖用户已有标准标签。
- 本计划只生成计划文档；实现阶段不自动 commit、push、merge 或 release。

## 官方模型合同

实现前固定以下官方资源和输出合同，转换后的 TFJS 文件必须保留这些语义：

| Head ID | 官方资源 | 类型 | 类别/输出 | 输入 |
| --- | --- | --- | --- | --- |
| `discogs_effnet_embedding` | `discogs-effnet-bs64-1` | embedding | `PartitionedCall:1`, 1280 维 | 16 kHz、`[64,128,96]` Mel |
| `discogs_mood_theme` | `mtg_jamendo_moodtheme-discogs-effnet-1` | multi-label | 56 类，`model/Sigmoid` | 1280 维 |
| `discogs_approachability` | `approachability_2c-discogs-effnet-1` | multi-class | `not approachable` / `approachable`，`model/Softmax` | 1280 维 |
| `discogs_instrumentation` | `mtg_jamendo_instrument-discogs-effnet-1` | multi-label | 40 类，`model/Sigmoid` | 1280 维 |
| `discogs_timbre` | `timbre-discogs-effnet-1` | multi-class | `bright` / `dark`，`model/Softmax` | 1280 维 |
| `discogs_danceability` | `danceability-discogs-effnet-1` | multi-class | `danceable` / `not_danceable`，`model/Softmax` | 1280 维 |

官方元数据还规定了 16 kHz 推理采样率、模型版本和类别顺序；实现不得仅依赖文件名猜测类别。参考：[EffNet embedding 元数据](https://essentia.upf.edu/models/feature-extractors/discogs-effnet/discogs-effnet-bs64-1.json)、[Mood/Theme 元数据](https://essentia.upf.edu/models/classification-heads/mtg_jamendo_moodtheme/mtg_jamendo_moodtheme-discogs-effnet-1.json)、[Approachability 元数据](https://essentia.upf.edu/models/classification-heads/approachability/approachability_2c-discogs-effnet-1.json)、[Instrumentation 元数据](https://essentia.upf.edu/models/classification-heads/mtg_jamendo_instrument/mtg_jamendo_instrument-discogs-effnet-1.json)、[Timbre 元数据](https://essentia.upf.edu/models/classification-heads/timbre/timbre-discogs-effnet-1.json)、[Danceability 元数据](https://essentia.upf.edu/models/classification-heads/danceability/danceability-discogs-effnet-1.json)。

## 数据合同

在 `app/src/analysis.ts` 中新增以下兼容 DTO；所有字段可选，旧 JSON 读取时视为缺失：

```ts
export type DiscogsEffnetHeadId =
  | 'moodTheme'
  | 'approachability'
  | 'instrumentation'
  | 'timbre'
  | 'danceability';

export type DiscogsEffnetHeadStatus =
  | 'completed'
  | 'model_missing'
  | 'failed'
  | 'cancelled'
  | 'timeout';

export type DiscogsEffnetHeadResult = {
  model: DiscogsEffnetHeadId;
  status: DiscogsEffnetHeadStatus;
  version: string;
  labels: AnalysisLabel[];
  scores: Record<string, number>;
  frameCount: number;
  threshold?: number;
  selectedClass?: string;
  selectedConfidence?: number;
  reason?: string | null;
};

export type DiscogsEffnetAnalysis = {
  embeddingModel: 'discogs-effnet-bs64-1';
  embeddingDimensions: 1280;
  inputShape: [number, number, number];
  heads: Partial<Record<DiscogsEffnetHeadId, DiscogsEffnetHeadResult>>;
};
```

`HighLevelAnalysis` 增加可选的 `discogsEffnet?: DiscogsEffnetAnalysis`。现有 `genre`、`style`、`mood`、`instrument` 不迁移到该对象，也不因为 Discogs head 缺失而改写为空。

---

### Task 1: 注册模型族和离线资源清单

**Files:**

- Modify: `app/src/analysis.ts`
- Modify: `app/src/analysis-worker-protocol.ts`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/essentia_model_import.rs`
- Modify: `scripts/prepare_essentia_tfjs_resources.py`
- Add: `src-tauri/resources/essentia-models/discogs_effnet_embedding.json` and `.bin`
- Add: `src-tauri/resources/essentia-models/discogs_mood_theme.json` and `.bin`
- Add: `src-tauri/resources/essentia-models/discogs_approachability.json` and `.bin`
- Add: `src-tauri/resources/essentia-models/discogs_instrumentation.json` and `.bin`
- Add: `src-tauri/resources/essentia-models/discogs_timbre.json` and `.bin`
- Add: `src-tauri/resources/essentia-models/discogs_danceability.json` and `.bin`
- Test: `src-tauri/src/essentia_model_import.rs`

**Interfaces:**

Extend `EssentiaModelSpec` with `embedding_family`, `input_shape`, and `input_width`. Register the six IDs from the official contract. Keep the existing MusiCNN IDs unchanged. The Tauri status DTO adds an optional `discogsEffnet` object with `embedding` and per-head booleans so older clients can still decode the response.

- [x] **Step 1: Write failing contract tests.**

Add Rust fixtures that assert the six IDs, the embedding output width `1280`, head input width `1280`, exact output units `56/2/40/2/2`, output names, and official class counts. Assert a graph with input width `200`, an output width mismatch, or a wrong output node is rejected before installation.

- [x] **Step 2: Extend the resource preparation script.**

Add an explicit Discogs input directory and output mapping to `scripts/prepare_essentia_tfjs_resources.py`. The script must stage all six pairs, validate every `weightsManifest` byte count, validate graph node names and shapes, validate class metadata counts, then replace only the staged pair. It must fail if any required pair is absent or malformed; no partial set may be installed.

- [x] **Step 3: Convert official frozen graphs offline.**

Use the pinned local TensorFlow.js conversion tool against the official PB/SavedModel files. Record the exact conversion command and source metadata in `src-tauri/resources/essentia-models/NOTICE.md`. Convert the embedding with output `PartitionedCall:1` and each head with its official prediction output. Do not download during app startup or analysis.

- [x] **Step 4: Run resource validation.**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml essentia_model_import
python3 scripts/prepare_essentia_tfjs_resources.py --help
```

Expected: all existing model-import tests and the new Discogs shape/class tests pass; malformed and incomplete pairs leave the previous resource set untouched.

---

### Task 2: Implement the Discogs Mel frontend and shared embedding

**Files:**

- Modify: `app/src/analysis.ts`
- Modify: `app/src/analysis.worker.ts`
- Modify: `app/src/analysis-worker-protocol.ts`
- Test: `app/src/analysis.test.ts`
- Test: `app/src/analysis-worker-client.test.ts`

**Interfaces:**

```ts
export type DiscogsEffnetMelProgress = {
  processedPatches: number;
  totalPatches: number;
};

export type DiscogsEffnetMelBatch = {
  values: Float32Array;
  batchSize: number;
  framesPerPatch: 128;
  melBands: 96;
  validPatches: number;
};

export async function computeDiscogsEffnetMelBatches(
  essentia: EssentiaInstance,
  signal: Float32Array,
  onProgress?: (progress: DiscogsEffnetMelProgress) => void,
): Promise<DiscogsEffnetMelBatch[]>;
```

- [x] **Step 1: Write the frontend shape tests.**

Use a deterministic fake Essentia frontend and a short signal to assert 96 mel bands, 128 frames per patch, a final zero-padded patch, correct `validPatches`, stable row order, and progress reaching `totalPatches`. Assert the function does not return the existing MusiCNN `187×96` shape.

- [x] **Step 2: Implement dedicated EffNet preprocessing.**

Use 16 kHz mono audio and the official EffNet spectrogram parameters. Do not call `computeMusiCnnMelRows` or `TensorflowInputMusiCNN`. Convert each Essentia vector to JS numbers before releasing it, yield to the Worker event loop at most every 32 patches, and release the frame generator in `finally`. Pad only the final patch and keep `validPatches` so aggregation can ignore padding.

- [x] **Step 3: Add one shared embedding execution.**

Load `discogs_effnet_embedding` once per song Worker. Run all patches in batches of 64, transfer only the final numeric embedding rows to the head runner, and dispose every input/output tensor in `finally`. Validate the runtime output width is exactly 1280 before running any head.

- [x] **Step 4: Add Worker progress and cancellation coverage.**

Emit `analyzingHighLevel` progress with `modelId: 'discogs_effnet_embedding'` while extracting/embedding and with each head ID while executing heads. A terminated Worker must not write a partial `discogsEffnet` object. Run:

```bash
pnpm --dir app test -- --run app/src/analysis.test.ts app/src/analysis-worker-client.test.ts
```

Expected: shape, release, transfer, old-job filtering, cancellation, and progress tests pass.

---

### Task 3: Run and aggregate the five Discogs heads

**Files:**

- Modify: `app/src/analysis.ts`
- Add: `app/src/discogs-effnet.ts`
- Test: `app/src/analysis.test.ts`
- Test: `app/src/discogs-effnet.test.ts`

**Interfaces:**

```ts
export function runDiscogsEffnetHeads(
  tf: TensorflowRuntime,
  embeddingRows: number[][],
  models: EssentiaModelFile[],
  options?: {
    onProgress?: (model: DiscogsEffnetHeadId) => void;
    validRows?: number;
  },
): Promise<DiscogsEffnetAnalysis>;
```

- [x] **Step 1: Add deterministic aggregation tests.**

For `moodTheme` and `instrumentation`, assert that the mean probability across valid patches is filtered at `0.35`, raw scores are retained, and at most eight display labels are returned in descending confidence order. For `approachability`, `timbre`, and `danceability`, assert mean Softmax probabilities select the highest class and retain both class scores.

- [x] **Step 2: Implement the multi-label heads.**

Run `discogs_mood_theme` and `discogs_instrumentation` against the shared `[N,1280]` tensor. Use the official 56/40 class order, threshold `0.35`, display cap `8`, and record `threshold: 0.35`. Preserve raw finite probabilities in `scores`; do not convert these outputs to a single Genre.

- [x] **Step 3: Implement the multi-class heads.**

Run `discogs_approachability`, `discogs_timbre`, and `discogs_danceability` with their official two-class order. Save `selectedClass`, `selectedConfidence`, and both raw class scores. Use `not approachable`, `approachable`, `bright`, `dark`, `danceable`, and `not_danceable` exactly as provided by the model metadata.

- [x] **Step 4: Isolate head failures.**

Wrap each head in its own `try/finally`. Missing resources create `status: 'model_missing'`; runtime exceptions create `status: 'failed'`; cancellation/timeout is propagated as `cancelled`/`timeout`. A failed Timbre head must not remove Mood/Theme or current Danceability.

- [x] **Step 5: Verify no duplicate embedding work.**

Use a fake TF runtime that counts model loads and `execute` calls. Assert one embedding load/execute pipeline and exactly five head loads, with all head inputs width 1280. Run:

```bash
pnpm --dir app test -- --run app/src/discogs-effnet.test.ts app/src/analysis.test.ts
```

---

### Task 4: Persist results and preserve old analysis

**Files:**

- Modify: `src/w4dj_library.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `app/src/analysis.ts`
- Modify: `src/sync.rs`
- Test: `tests/w4dj_library.rs`
- Test: `tests/history.rs`
- Test: `src-tauri/src/main.rs` unit tests

**Interfaces:**

Add optional W4DJ projection fields with empty/default values for old databases:

```sql
discogs_mood_theme_json TEXT NOT NULL DEFAULT '[]',
discogs_approachability_json TEXT NOT NULL DEFAULT '{}',
discogs_instrumentation_json TEXT NOT NULL DEFAULT '[]',
discogs_timbre_json TEXT NOT NULL DEFAULT '{}',
discogs_danceability_json TEXT NOT NULL DEFAULT '{}'
```

The canonical full result remains `analysis_results.analysis_json`; projection fields only support query and Dashboard rendering.

- [x] **Step 1: Write migration and isolation tests.**

Open a pre-existing database without the new columns and migrate it. Assert old MusiCNN Style, Mood, Instrument and Danceability values remain unchanged. Apply a Discogs result and assert only the five new columns and `analysis_json` change. Apply a head failure and assert its prior successful value is retained unless the user explicitly reruns and receives a new completed value.

- [x] **Step 2: Add safe projection extraction.**

Extract `highLevel.discogsEffnet.heads` by stable IDs, serialize labels/scores deterministically, and update the same `destination_path` transaction as the existing analysis. Old JSON with no `discogsEffnet` must read as `model_missing`, not as a failed analysis.

- [x] **Step 3: Add namespaced audio metadata.**

Extend `src/sync.rs` with `W4DJ-Discogs-MoodTheme`, `W4DJ-Discogs-Approachability`, `W4DJ-Discogs-Instrumentation`, `W4DJ-Discogs-Timbre`, and `W4DJ-Discogs-Danceability` TXXX fields plus the readable summary. Do not replace `W4DJ-Danceability`, `TBPM`, `TKEY`, or an existing user Genre. Re-read and validate every written field; if validation fails, preserve the previous stored result.

- [x] **Step 4: Test compatibility and report fields.**

Run:

```bash
cargo test --test w4dj_library
cargo test --test history
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: old records remain readable, each head has an independent status, and a cancelled/failed Discogs run cannot erase an existing successful analysis.

---

### Task 5: Dashboard detail, optional columns, and query filters

**Files:**

- Modify: `app/src/library-dashboard.ts`
- Modify: `app/src/library-dashboard.test.ts`
- Modify: `src/library_query.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/w4dj_library.rs`

**Interfaces:**

Add optional `LibraryTrack` fields:

```ts
discogsMoodThemeJson: string;
discogsApproachabilityJson: string;
discogsInstrumentationJson: string;
discogsTimbreJson: string;
discogsDanceabilityJson: string;
```

Add query fields `discogs_mood_theme`, `discogs_approachability`, `discogs_instrumentation`, `discogs_timbre`, and `discogs_danceability` to the Rust/TypeScript whitelist. Unknown fields must still be rejected.

- [x] **Step 1: Write rendering and filter tests.**

Render a track containing all five results and assert the detail drawer shows model source, selected label, confidence, and missing/failed status without `null` or `undefined`. Assert the table can render optional columns for Mood/Theme, Approachability, Instrumentation, Timbre, and Discogs Danceability. Assert old three-column cleanup remains intact. Test text `contains`, exact class, and numeric confidence filters.

- [x] **Step 2: Implement compact display helpers.**

Use badges for Mood/Theme and Instrumentation, a single selected class for Approachability/Timbre, and `danceable 82%` plus raw class scores for Discogs Danceability. Keep all five optional columns hidden by default to avoid restoring the removed table clutter; the detail drawer remains the complete view.

- [x] **Step 3: Implement query projection.**

Join the five projection fields from W4DJ SQLite, add stable sorting and filtering, and ensure pagination totals are calculated after filters. Missing head results must sort after completed values and display a localized `未生成`/`Not generated` marker.

- [x] **Step 4: Run frontend and Rust query tests.**

```bash
pnpm --dir app test -- --run app/src/library-dashboard.test.ts
cargo test --test library_catalog
cargo test --test w4dj_library
```

---

### Task 6: Analysis summaries, manual reports, and model status

**Files:**

- Modify: `app/src/app.ts`
- Modify: `src/history.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `app/src/analysis-worker-protocol.ts`
- Test: `app/src/app.test.ts`
- Test: `tests/history.rs`

- [x] **Step 1: Add per-head status to progress and history.**

Progress messages include `modelFamily: 'discogsEffnet'`, `modelId`, `stage`, `processed`, and `total`. The analysis summary counts Discogs head completed/model-missing/failed/timeout without changing the song-level conversion or analysis status.

- [x] **Step 2: Separate model missing from analysis failure.**

When the embedding is missing, report one `model_missing` Discogs group and keep base analysis. When one head is missing, report only that head as missing and keep other successful heads. Do not automatically retry or download resources.

- [x] **Step 3: Extend manual exports.**

The manually selected error report and run-session export include a `[Discogs-EffNet 逐 head 状态]` section with model ID, version, selected labels/classes, confidence, frame count, elapsed time, and reason. Do not auto-create or overwrite reports.

- [x] **Step 4: Test report completeness.**

Construct sessions containing all five head states (`completed`, `model_missing`, `failed`, `timeout`, `cancelled`) and assert every state appears in the manually exported report while conversion completion remains separate from analysis completion.

---

### Task 7: End-to-end acceptance and documentation

**Files:**

- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`
- Modify: `src-tauri/resources/essentia-models/NOTICE.md`
- Add: `docs/testing/discogs-effnet-acceptance.md`

- [x] **Step 1: Run automated verification.**

```bash
pnpm --dir app test -- --run
pnpm --dir app build
cargo test --all
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Record the actual test counts, direct-command fallbacks if the local pnpm wrapper is blocked, and the known root-workspace legacy `dead_code` Clippy result. Do not claim strict root Clippy success if it remains blocked by the existing baseline.

- [ ] **Step 2: Run real-output acceptance.**

Use the existing nine output audio files. Verify one shared embedding is produced per song, all five head statuses are present, and one failed/missing head does not remove other results. Check that a cancelled run retains prior fields and that a rerun updates the same `destination_path` record.

- [ ] **Step 3: Validate metadata and DJ-facing fields.**

With ExifTool, verify the five namespaced TXXX fields and comment after a successful write. Verify `W4DJ-Danceability` remains the existing Essentia value and `W4DJ-Discogs-Danceability` is separate. Confirm existing Genre, BPM and Key are not overwritten. Test MP3, FLAC, WAV and AIFF where available.

- [x] **Step 4: Record product limitations.**

Document that Mood/Theme and Instrumentation are multi-label predictions, Approachability is an inferred listener-accessibility category, Timbre is only bright/dark, and Discogs Danceability is a second model source rather than a replacement for the calibrated ten-level value. Record the absence of Windows/Rekordbox/real-user GUI validation when those environments are unavailable.

## 执行记录（2026-08-23）

Tasks 1–6 的实现与自动化步骤已完成。前端直接 Vitest 为 7 个测试文件、146/146 通过；Tauri 单元测试 47/47 通过；根工作区 `cargo test --all` 412 个测试用例通过；`cargo fmt --all -- --check`、Tauri `cargo check`、Tauri all-targets Clippy 和 `git diff --check` 通过；Vite 生产构建通过。当前 pnpm 包装命令受本机 `ignored build scripts` 安全策略阻断，已使用同一安装的 Node/Vitest 与 Vite 直接命令完成等价验证。根工作区严格 all-targets Clippy 仍仅被既有 legacy `dead_code` 警告阻断，并已用 `-A dead_code -D warnings` 验证新代码无 Clippy 警告。

Task 7 的真实输出重分析、ExifTool 回读、MP3/FLAC/WAV/AIFF 全格式、Windows、Rekordbox、浏览器盲听及 100/200 首人工评测依赖外部素材或人工环境，本次未宣称完成；对应步骤保留未勾选，人工步骤见 `docs/testing/discogs-effnet-acceptance.md`。

## Acceptance Criteria

- One completed analysis contains independent results for Mood/Theme, Approachability, Instrumentation, Timbre and Discogs Danceability under `highLevel.discogsEffnet`.
- The five heads share one 1280-dimensional Discogs-EffNet embedding and never send Discogs tensors through the MusiCNN `[N,200]` path.
- Official input/output shapes, class order, output node names, manifest lengths and resource bytes pass strict offline validation.
- A missing or failed head is reported independently and does not erase existing analysis or block basic analysis.
- Existing Style, Mood, Instrument, Energy, original Danceability, NetEase metadata and user tags remain unchanged.
- Dashboard detail and filters expose all five new results; optional columns are available but hidden by default.
- Manual error reports include per-head status and do not get auto-generated or overwritten.
- ExifTool can read the namespaced Discogs fields after a successful metadata write, and failed/cancelled runs do not overwrite them.
- Automated tests, build, format, check, Clippy and `git diff --check` are recorded truthfully; no version change, commit, push, merge or release occurs during implementation.
