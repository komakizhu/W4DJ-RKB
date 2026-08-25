# 三套情绪模型后台接入与四模型主观验收实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use 执行计划代理 or executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking. Do not commit, push, merge, release, or modify the version.

> **2026-08-24 验收入口更新：** 模型重分析、manifest 生成和机器校验改用 `2026-08-24-headless-acceptance.md` 的后台场景，不打开 W4DJ App GUI。主观盲听仍由人完成，但使用独立验收页面，不属于 W4DJ App GUI 验收。

**Goal:** 在保留现有五个 Mood 头作为旧基线的前提下，接入 emoMusic、MuSe、MIREX 三套情绪模型，将四套结果写入后台分析和验收 manifest，并用现有独立 HTML 工具完成 5、20、100/200 首歌曲的盲测，最后再决定哪些字段公开到 Dashboard。

**Architecture:** 现有 MusiCNN embedding 仍只负责产生共享的 200 维嵌入；五个 mood_* 二分类头继续组成 legacyMood 基线，三个新 head 在同一个 Worker 中独立执行。emoMusic 与 MuSe 各保存自己的 Valence/Arousal 1–9 坐标，MIREX 保存五个情绪簇，三者不平均、不覆盖、不塞入旧 mood 数组。Rust 只负责严格校验随包模型、序列化结果和生成只读验收 manifest；Dashboard 在主观验收完成前不增加默认情绪列。

**Tech Stack:** TypeScript/Vite/Vitest、TensorFlow.js GraphModel、Essentia.js Worker、Rust 2024/Tauri 2、serde、rusqlite、现有 w4dj.sqlite3、原生 HTML/CSS/ES modules、IndexedDB。

## 当前执行状态

Task 1、Task 3、Task 4 的代码与自动化验收已完成。Task 2 的三个官方 head 已从本地官方 ONNX 资源离线转换、重新读取校验并随 Tauri 资源打包；没有占位模型或运行时下载。当前已有 10 条可用输出，但历史分析 JSON 尚未自动重跑，因此旧 manifest 中仍可能显示 `model_missing`，需用户重新分析当前输出后才能进行真实盲测。静态 HTTP 服务已完成本地 curl 冒烟验证；浏览器播放、主观盲听和公开字段决策仍必须由人工完成，100/200 首也受实际可用歌曲数量限制，不用重复歌曲伪造样本。

## Global Constraints

- 四个比较系统固定为 legacyMood（五个既有 Mood 头的聚合基线）、emomusic、muse、mirex；五个旧头不是五个额外竞争系统。
- style 继续承载 MusiCNN 50 标签（包括 80s、House、electronic 等）；本计划不实现 Discogs-EffNet genre，也不把宽 Genre 结果冒充新 Genre。
- emomusic 与 muse 分别输出 Valence/Arousal，值域必须为 [1, 9]；mirex 输出五个固定情绪簇；已有五个 Mood 标签继续写入 mood。
- 三个新 head 缺失、损坏、失败或取消时只标记自己的状态；基础分析、Style、旧 Mood 和其他成功 head 必须保留。
- 模型只能从 src-tauri/resources/essentia-models/ 离线加载；运行时不下载，不静默切换到主线程推理，不使用伪造输出。
- 模型安装前后都要重新读取并校验 graph、manifest、输入宽度、输出层、输出维度和权重长度；不通过校验的文件不得参与真实分析。
- 继续使用当前 Worker 的 jobId/requestId 协议；旧消息、终止后的结果和半成品不得写入缓存或 SQLite。
- 新字段先进入后台 JSON、analysis_results 和验收 manifest；真实盲测完成前不新增 Dashboard 默认列、筛选项、音频回写规则或公开设置。
- 不修改版本号，不新增网络依赖，不创建 hash、baseline、冻结 contract 或发布 gate，不 commit、push、merge、release。

## 模型定义与官方依据

| 验收 ID | 代码字段 | 输入/输出 | 用途 |
| --- | --- | --- | --- |
| legacyMood | highLevel.mood | 五个二分类 head：aggressive/happy/relaxed/party/sad | 现有 W4DJ 基线；每个正类标签独立保留 |
| emomusic | highLevel.emotionCandidates.emomusic | 共享 200 维 embedding，两个连续值 valence、arousal，范围 1–9 | emoMusic/MSD 情绪坐标 |
| muse | highLevel.emotionCandidates.muse | 共享 200 维 embedding，两个连续值 valence、arousal，范围 1–9 | MuSe/MSD 情绪坐标 |
| mirex | highLevel.moodCluster | 共享 200 维 embedding，五个固定簇的置信度 | MIREX 情绪簇 |

模型资源和输出名称以 Essentia model documentation（https://essentia.upf.edu/models.html）为准：emoMusic 使用 emomusic-msd-musicnn-2、输出 model/Identity；MuSe 使用 muse-msd-musicnn-2、输出 model/Identity；MIREX 使用 moods_mirex-msd-musicnn-1、输出 PartitionedCall。五簇标签固定为：passionate/rousing/confident/boisterous/rowdy、rollicking/cheerful/fun/sweet/amiable、literate/poignant/wistful/bittersweet/autumnal/brooding、humorous/silly/campy/quirky/whimsical/witty/wry、aggressive/fiery/tense/anxious/intense/volatile/visceral。

## 文件地图

- app/src/emotion-models.ts：新建 Worker-safe 的三个 head 的规格、输出归一化、独立执行和错误隔离；不访问 DOM、IndexedDB 或 Tauri。
- app/src/emotion-models.test.ts：固定 Tensor 输出、值域、五簇映射、缺失/失败/取消隔离测试。
- app/src/analysis.ts：扩展 HighLevelAnalysis、模型 ID/文件类型和高层编排；保留现有基础分析与五个 Mood 头。
- app/src/analysis-worker-protocol.ts、app/src/analysis.worker.ts、app/src/analysis-worker-client.ts：传递新增高层结果和按 head 的进度，继续过滤旧 jobId。
- app/src/analysis.test.ts、app/src/analysis-worker-client.test.ts、app/src/app.test.ts：兼容旧 JSON、Worker 结果、取消和失败回写回归。
- src/analysis.rs：Rust HighLevelAnalysis、连续情绪结果和 MIREX 簇的 camelCase 可选字段。
- src-tauri/src/essentia_model_import.rs：唯一模型注册表、离线资源导入和严格结构/权重校验；不重复接受路径伪装模型。
- src-tauri/src/main.rs：加载内置/导入模型时使用同一严格校验，向 Worker/前端提供新增资源。
- src-tauri/resources/essentia-models/：只放重新读取校验通过的三个 head 的 model.json 与权重 shard，并更新 NOTICE 的实际资源清单。
- scripts/prepare_essentia_tfjs_resources.py：离线转换、暂存、重新读取和校验模型；转换失败必须报告，不能生成占位模型。
- src/w4dj_library.rs、tests/w4dj_library.rs：保存新 high-level JSON 并为验收 manifest 映射四套状态/结果；不改变 Dashboard 数据源边界。
- tools/emotion-evaluation/：复用现有 HTML 工具；只补充结果摘要和模型输出展示，不把它并入正式 Dashboard。
- 计划.md、docs/project-state.md、docs/handoff.md、docs/superpowers/plans/2026-08-22-genre-style-emotion-models.md：记录本计划各步、自动化验证和人工盲测结果；Discogs 相关步骤继续保持未完成。

---

### Task 1: 固定兼容结果契约

**Files:**
- Modify: app/src/analysis.ts
- Modify: src/analysis.rs
- Modify: app/src/analysis.test.ts
- Modify: tests/w4dj_library.rs

**Interfaces:**

~~~ts
export type EmotionHeadStatus = 'completed' | 'model_missing' | 'failed' | 'cancelled';

export type ContinuousEmotionResult = {
  model: 'emomusic' | 'muse';
  status: EmotionHeadStatus;
  valence: number | null;
  arousal: number | null;
  reason?: string | null;
};

export type EmotionCandidates = {
  emomusic?: ContinuousEmotionResult;
  muse?: ContinuousEmotionResult;
};

export type HighLevelAnalysis = {
  status: 'completed' | 'model_missing' | 'failed';
  modelVersion?: string | null;
  reason?: string | null;
  genre?: AnalysisLabel[];
  style?: AnalysisLabel[];
  mood?: AnalysisLabel[];
  instrument?: AnalysisLabel[];
  emotionCandidates?: EmotionCandidates;
  moodCluster?: AnalysisLabel[];
  filtered?: Array<{ label: string; confidence: number; reason: string }>;
};
~~~

Rust 对应字段使用 serde rename_all = camelCase，新增 style、emotion_candidates、mood_cluster 和 ContinuousEmotionResult 的字段全部 serde(default)，确保旧版 track-analysis.json 能读取。completed 时 valence/arousal 必须为有限 1–9 数值；其他状态两值必须为 null 并提供可显示的 reason。

- [x] Step 1: 写旧 JSON 兼容失败测试

在 app/src/analysis.test.ts 和 tests/w4dj_library.rs 使用没有新增字段的旧 high-level JSON，断言解析后 style、emotionCandidates、moodCluster 为空或缺省且既有五个 Mood 标签不变；再用 camelCase 新 JSON 断言字段完整往返。

- [x] Step 2: 实现 TypeScript/Rust DTO

扩展 HighLevelAnalysis 和 Rust 结构，新增 EmotionHeadStatus、ContinuousEmotionResult、EmotionCandidates，为反序列化失败保留已有基础分析字段，不将新增字段的异常升级为整首歌曲失败。

- [x] Step 3: 运行契约测试

Run:
~~~bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH \
  pnpm --dir app test -- --run app/src/analysis.test.ts
cargo test --test w4dj_library
~~~

Expected: 旧 JSON、新 JSON、camelCase 和五个 Mood 头回归全部通过；不存在 undefined 被序列化成伪造的完成结果。

### Task 2: 注册并离线安装三个模型 head

**Files:**
- Modify: app/src/analysis.ts
- Modify: src-tauri/src/essentia_model_import.rs
- Modify: src-tauri/src/main.rs
- Modify: scripts/prepare_essentia_tfjs_resources.py
- Add: src-tauri/resources/essentia-models/emomusic-msd-musicnn-2/model.json 和权重 shard
- Add: src-tauri/resources/essentia-models/muse-msd-musicnn-2/model.json 和权重 shard
- Add: src-tauri/resources/essentia-models/moods_mirex-msd-musicnn-1/model.json 和权重 shard
- Test: src-tauri/src/essentia_model_import.rs

**Interfaces:**

~~~ts
export type EmotionModelId = 'emomusic' | 'muse' | 'mirex';

export type EssentiaModelSpec = {
  id: string;
  kind: 'embedding' | 'mood' | 'instrument' | 'emotionContinuous' | 'emotionCluster';
  inputWidth: 200 | null;
  outputUnits: number;
  outputName: string;
  classes: readonly string[];
  version: string;
};
~~~

emomusic 和 muse 的 outputUnits 为 2、outputName 为 model/Identity；mirex 的 outputUnits 为 5、outputName 为 PartitionedCall。Rust KNOWN_MODELS 与前端规格必须由同一组 ID、shape、输出名称驱动，不能继续只靠路径 marker 识别。

- [x] Step 1: 写资源拒绝测试

在 src-tauri/src/essentia_model_import.rs 增加 fixtures：错误 input width、错误 output units、错误 output node、缺 shard、manifest 声明长度与实际 BIN 不符、重复模型 ID。断言导入失败、旧 pair 不被覆盖，并接受包含正确 MusiCNN 输入节点、输出节点和权重长度的三个最小有效 fixture。

- [x] Step 2: 扩展唯一模型规格和导入器

将三个 ID 加入模型规格；导入过程按 model.json 所在目录和 manifest 的相对路径精确配对，重新读取暂存 pair 后验证 topology、输入宽度 200、输出层名称、输出维度、class 数量和每个 shard 的字节数。任何一个 pair 失败都返回明确 issue，不安装该 pair。

- [x] Step 3: 离线转换并安装资源

使用 `scripts/prepare_essentia_tfjs_resources.py` 处理本地官方资源，写入临时目录后重新读取校验，再原子替换到 `src-tauri/resources/essentia-models/`。emoMusic/MuSe 使用官方 ONNX 导出，MIREX 使用官方 ONNX 导出并保留五簇输出；三个 pair 的输入宽度、输出节点、输出维度和权重长度均已通过 Rust 导入器。不得用空 JSON、随机权重或运行时网络下载伪造完成。

- [x] Step 4: 运行资源专项测试

Run:
~~~bash
cargo test --manifest-path src-tauri/Cargo.toml essentia_model_import -- --nocapture
cargo check --manifest-path src-tauri/Cargo.toml
~~~

Expected: 现有内置模型和三个新增 head 的严格校验通过；损坏、冒名、不匹配资源被拒绝；普通转换不因新 head 缺失而失败。

### Task 3: 在 Worker 中独立执行三个 head

**Files:**
- Create: app/src/emotion-models.ts
- Create: app/src/emotion-models.test.ts
- Modify: app/src/analysis.ts
- Modify: app/src/analysis-worker-protocol.ts
- Modify: app/src/analysis.worker.ts
- Modify: app/src/analysis-worker-client.ts
- Modify: app/src/analysis.test.ts
- Modify: app/src/analysis-worker-client.test.ts

**Interfaces:**

~~~ts
export type EmotionRunOptions = {
  isCancelled?: () => boolean;
  onProgress?: (model: EmotionModelId) => void;
};

export type EmotionHeadRun = {
  emotionCandidates: EmotionCandidates;
  moodCluster: AnalysisLabel[];
  failures: Array<{ model: EmotionModelId; reason: string }>;
};

export async function runEmotionHeads(
  tf: any,
  embeddingRows: readonly number[][],
  models: ReadonlyMap<string, EssentiaModelFile>,
  options?: EmotionRunOptions,
): Promise<EmotionHeadRun>;
~~~

- [x] Step 1: 写 Worker-safe 失败测试

使用 fake execute() 模型固定输出：emoMusic/MuSe 各返回不同的两个数，MIREX 返回五个分数；断言数值分别保存、MIREX 只生成五簇、五个旧 Mood 标签仍可同时存在。再让其中一个 head throw、另一个 head 缺失和取消发生，断言其他 head、基础分析和旧缓存不被清空。

- [x] Step 2: 实现三个 head 的归一化和执行

新增 emotion-models.ts，把 embedding batch 送入各自 outputName；对连续输出按帧求均值并验证有限值/[1,9]，对 MIREX 输出按固定五簇 class 顺序映射 AnalysisLabel。每个 head 单独 try/catch，失败返回自己的状态；每次 head 前后检查 isCancelled，Tensor 在 finally 中 dispose。

- [x] Step 3: 接入现有高层编排

保留 musicnn_embedding、Style 标签和五个 Mood head 的原顺序；在 embedding 得到后调用 runEmotionHeads。新 head 缺失时 HighLevelAnalysis.status 仍可为 completed，只在对应字段写 model_missing；只有共享 embedding/基础流程失败才沿用整段 high-level 失败逻辑。

- [x] Step 4: 完善 Worker 消息和取消测试

进度 payload 增加可选 modelId，仍携带 jobId/requestId；客户端收到旧 job 消息直接丢弃。终止 Worker 时不提交当前歌曲结果，重新开始创建新 Worker；测试覆盖 start、progress、result、error、旧 job 过滤和再次启动。

- [x] Step 5: 运行前端专项测试

Run:
~~~bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH \
  pnpm --dir app test -- --run \
  app/src/emotion-models.test.ts \
  app/src/analysis.test.ts \
  app/src/analysis-worker-client.test.ts
~~~

Expected: 四套后台结果互不覆盖，取消/旧 job 不写半成品，旧五 Mood 回归通过，UI 不因新 head 失败而失去基础结果。

### Task 4: 持久化 high-level 字段并扩展验收 manifest

**Files:**
- Modify: src/analysis.rs
- Modify: src/w4dj_library.rs
- Modify: tests/w4dj_library.rs
- Modify: src-tauri/src/main.rs
- Modify: tools/emotion-evaluation/evaluator.js
- Modify: tools/emotion-evaluation/main.js
- Modify: tools/emotion-evaluation/main.test.js

**Interfaces:**

~~~rust
pub fn save_track_analysis(&mut self, entry: &TrackAnalysis) -> W4djResult<()>
pub fn emotion_evaluation_manifest(
    &self,
    count: usize,
    seed: u64,
) -> W4djResult<EmotionEvaluationManifest>
~~~

analysis_results.analysis_json 继续保存完整 TrackAnalysis；不新增第二个分析数据库。manifest 中 legacyMood 从 highLevel.mood、emomusic/muse 从 highLevel.emotionCandidates、mirex 从 highLevel.moodCluster 提取；缺字段写 status: model_missing，不把 [] 当成成功。

- [x] Step 1: 写回归 fixture

在 tests/w4dj_library.rs 写入一条同时有五个 Mood、两个连续坐标和五个 MIREX 簇的 TrackAnalysis，读取 manifest 后断言四个 model card 都是 completed 且值不串位；再写入单个 head 失败 JSON，断言其它 card 可用、失败 card 不进入计分分母。

- [x] Step 2: 实现 Rust 写回和 manifest 映射

扩展 serde 结构和现有 upsert/analysis 写回，不改变 destination path 绑定；保持 track-analysis.json 兼容镜像。manifest 继续只选择 available 输出，保留 clipStartSeconds、clipDurationSeconds、clipSelection 和固定 seed。

- [x] Step 3: 显示四套后台输出但保持盲测匿名

验收页面的结果卡只显示规范化输出：旧基线显示 Mood 标签，emoMusic/MuSe 显示“愉悦度/激烈度”，MIREX 显示簇标签；卡片顺序仍用 seed 随机化，页面不显示真实 model ID，导出的 JSON 才保存映射。

- [x] Step 4: 运行存储和工具测试

Run:
~~~bash
cargo test --test w4dj_library
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH \
  pnpm --dir app test -- --run \
  tools/emotion-evaluation/evaluator.test.js \
  tools/emotion-evaluation/main.test.js
~~~

Expected: 新旧 JSON 均可读，四套结果映射正确，页面对缺失/失败模型禁用选择且不改变 W4DJ 数据库。

### Task 5: 执行分层真实歌曲盲测并作出公开字段决策

**Files:**
- Read/Use: tools/emotion-evaluation/README.md
- Generate: /private/tmp/w4dj-emotion-evaluation-manifest.json
- Generate: /private/tmp/w4dj-emotion-evaluation-5.json、20.json、100-or-200.json
- Record: docs/testing/emotion-model-evaluation-2026-08-22.md
- Modify after evidence: 计划.md、docs/project-state.md、docs/handoff.md

- [x] Step 1: 生成当前输出的 manifest（使用数据库副本；5/20/100 请求实际得到 5/10/10 首）

使用当前 W4DJ 数据库和固定 seed 导出 5 首夹具；先检查 manifest 的 sampleSize、四套状态、相对路径和 10 秒片段字段，再用同一 seed 重新导出确认顺序稳定。

~~~bash
CARGO_TARGET_DIR=/private/tmp/w4dj-emotion-acceptance-target \
  cargo run --quiet --manifest-path src-tauri/Cargo.toml \
  --bin export_emotion_evaluation_manifest -- \
  --database "$W4DJ_DB" \
  --output /private/tmp/w4dj-emotion-evaluation-5.json \
  --count 5 --seed 20260822
~~~

- [ ] Step 2: 用 5 首完成工具流程验收（模型资源已就绪；仍需人工浏览器播放与主观选择）

在本地静态服务器打开 tools/emotion-evaluation/index.html，选择 manifest 和实际输出目录；每首只听固定 10 秒，先选主观标签，再在匿名 A/B/C/D 中选择、并列或都不符合。刷新页面恢复会话，导出 JSON/CSV，检查四套状态、有效分母和 seed。

- [x] Step 3: 扩大到 20 首并核对数据链（生成 10 首 manifest；未执行浏览器听测）

重复导出 20 首，抽查至少一条每个状态组合；同时检查 w4dj.sqlite3、track-analysis.json 和 analysis_results.analysis_json，确认验收工具只读，不写分析结果、历史或音频标签。

- [ ] Step 4: 完成 100/200 首盲测（当前只有 10 条实际可用输出，需补充真实输出后再执行，不重复歌曲）

在真实输出数量允许时使用 100 首，否则使用全部可用歌曲且在报告中记录实际数量。报告每个模型的有效样本数、胜率/并列分数、按主观情绪的分组结果、缺失/失败数量和无法播放数量；不以少于两套可用模型的歌曲计入胜率。

- [ ] Step 5: 根据盲测证据决定公开字段（人工盲测完成前保持新增字段后台存储、不进入 Dashboard）

只有报告完成后，才在 docs/testing/emotion-model-evaluation-2026-08-22.md 明确选择：继续隐藏、公开 emomusic/muse 坐标、公开 MIREX 簇、保留五个 Mood，或组合其中几项。未被选中的字段仍保留后台数据，不能删除或覆盖；本步骤不直接修改 Dashboard，除非另行批准。

### Task 6: 完成回归、文档和交付核对

**Files:**
- Modify: 计划.md
- Modify: docs/project-state.md
- Modify: docs/handoff.md
- Modify: docs/superpowers/plans/2026-08-22-genre-style-emotion-models.md

- [x] Step 1: 更新计划状态

仅将本计划已完成的情绪字段、Worker、资源、manifest 和人工盲测步骤标为 [x]；Discogs-EffNet Genre 步骤继续标为 [ ]，不得用计划文字宣称尚未执行的真实模型或人工验收已完成。

- [x] Step 2: 运行完整自动验证

Run:
~~~bash
cargo test --all
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --lib --all-features -- -D warnings
cargo clippy --all-targets --all-features -- -A dead_code -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -A dead_code -D warnings
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH \
  pnpm --dir app test -- --run
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH \
  pnpm --dir app build
git diff --check
git status --short --branch
git diff --stat
~~~

Expected: Rust、前端、模型导入、Worker、manifest 和 HTML 工具自动测试通过；若严格 all-targets 仍只出现 AGENTS 已记录的既有 dead_code 基线，要在文档中如实保留；构建不发生网络模型下载。

- [x] Step 3: 记录不能自动完成的环境项

若缺少真实网易云数据库、官方 MIREX 转换源、macOS GUI、实际输出音频或人工听测环境，只记录未执行原因和复现命令；不把人工步骤改成伪自动测试，也不因此删除后续可执行的自动验证。

## Acceptance Criteria

- 旧版分析 JSON、旧五个 Mood 头和普通转换路径完全兼容。
- 一首歌的 high-level 结果可以同时包含五个旧 Mood 标签、style、emoMusic 坐标、MuSe 坐标和五个 MIREX 簇；任何单个新增 head 失败不会清空其它结果。
- 三个新增模型均来自应用内置、严格校验的资源；资源缺失时结果明确为 model_missing，普通转换和基础分析仍成功。当前资源已随包提供，旧分析记录不会自动伪造新结果，必须重新分析输出后才进入完成分母。
- Worker 分析期间取消立即终止，旧 jobId 结果被忽略，当前歌曲不写半成品；成功结果绑定同一 destination path。
- 5、20、100/200 首 HTML 盲测可暂停、恢复、随机展示四套匿名卡片，并导出可复核 JSON/CSV；缺失模型和不可播放音频不进入对应分母。
- 在主观报告完成前，Dashboard 不新增默认情绪列；报告完成后只按记录证据选择公开字段，未公开字段仍保留后台数据。
- 所有自动测试、构建、fmt/check/clippy、diff 检查结果和真实人工限制均写入项目状态/交接文档；不修改版本号、不 commit、不 push、不发布。
