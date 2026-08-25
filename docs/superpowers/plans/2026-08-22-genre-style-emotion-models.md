# Genre/Style 双轨与情绪模型扩展实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `执行计划代理` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Do not dispatch subagents from a side conversation.

**Goal:** 在不破坏现有基础分析和网易云元数据的前提下，把 MusiCNN 多标签作为 `Style`，把 Discogs-EffNet 细分风格作为 `Genre`，并先将 emoMusic、MuSe、MIREX 三套情绪能力完整接入后台数据与测试，待真实歌曲评测后再决定公开哪些字段。

**Architecture:** MusiCNN 继续作为现有 200 维嵌入和 50 标签来源；同一嵌入上的五个既有 Mood 二分类头、emoMusic、MuSe 和 MIREX 头作为彼此独立的可选 head。新增三套情绪结果先进入 Worker、SQLite 和兼容 JSON 的后台字段，不进入 Dashboard 默认列。Discogs-EffNet 使用独立的 embedding family、输入形状和 1280 维输出，不复用当前固定 `[N, 200]` 路径。所有同步 Essentia/TensorFlow 推理继续在 Worker 中执行，Dashboard 只读取 W4DJ SQLite 的结构化投影。

**Tech Stack:** TypeScript/Vite/Vitest、TFJS GraphModel、Essentia.js Worker、Rust/Tauri 模型资源校验、rusqlite/W4DJ SQLite。

## 当前执行状态

Style 与三个后台情绪 head 的 DTO、Worker 隔离、严格导入校验、SQLite/JSON 兼容字段和验收 manifest 已完成；emoMusic、MuSe、MIREX 官方资源已离线内置并通过 TFJS/Rust 校验。Discogs-EffNet 共享 embedding 与 `genre_discogs400` Genre pipeline 已接入现有 `HighLevelAnalysis.genre`，不会回退为 Style；五个额外 Discogs head、Dashboard 默认公开列和人工盲测仍未完成。旧分析记录不会因安装模型自动重跑。详细执行记录见 `docs/superpowers/plans/2026-08-22-emotion-model-backends-and-acceptance.md`。

## Global Constraints

- `neteaseGenre`、`genre` 和 `style` 永远分列；网易云 Genre 不得被模型结果覆盖。
- `genre` 是 Discogs-EffNet 细分主风格及候选；`style` 是 MusiCNN 50 标签的多标签数组，`80s` 等年代/属性标签必须保留。
- 旧 `essentia_genre` 投影只迁移为 `style`，不能在没有 Discogs 推理时冒充新 `genre`。
- `emotionCandidates.emomusic`、`emotionCandidates.muse` 和 `moodCluster` 三套新增情绪结果先全部作为后台测试字段保存；真实歌曲评测后再选择公开字段和随包内置模型。
- `emomusic` 与 `muse` 各自输出 1–9 的 Valence/Arousal，必须分别保存，不平均、不覆盖；`moods_mirex` 输出五簇，必须与连续坐标和现有 Mood Tags 分开。
- 现有 `mood_aggressive`、`mood_happy`、`mood_relaxed`、`mood_party`、`mood_sad` 五个 Mood 二分类头继续兼容并可并行运行；它们的正类标签可以同时存在。
- 新增情绪模型已作为严格校验通过的可选内置资源接入；运行时不得从网络下载模型，未通过离线校验的模型不得参与真实分析。
- 基础 BPM、Key、Loudness、Energy、Essentia 原始 Danceability 和十级展示不改变。
- 模型缺失、推理失败或取消不得覆盖旧结果；取消当前歌曲不写入半成品字段。
- 模型转换后必须重新读取并验证 graph、manifest、输入输出形状和权重长度；不从网络运行时下载模型。
- 不修改版本号，不 commit、push、merge 或 release。

---

### Task 1: 结果类型和双轨命名

**Files:**
- Modify: `app/src/analysis.ts`
- Modify: `app/src/analysis-worker-protocol.ts`
- Modify: `app/src/analysis.test.ts`
- Modify: `src-tauri/src/main.rs`
- Modify: `tests/w4dj_library.rs`

**Interfaces:**
- `HighLevelAnalysis.genre: AnalysisLabel[]`：Discogs 主风格/候选。
- `HighLevelAnalysis.style: AnalysisLabel[]`：MusiCNN 50 标签。
- `HighLevelAnalysis.emotionCandidates?: { emomusic?: { model: 'emomusic'; valence: number; arousal: number }; muse?: { model: 'muse'; valence: number; arousal: number } }`：两个连续情绪模型的后台对照结果，暂不映射为单一正式 Emotion。
- `HighLevelAnalysis.moodCluster?: AnalysisLabel[]`：MIREX 五簇，独立于 `mood`。

- [x] **Step 1: 写字段回归测试**

在 `app/src/analysis.test.ts` 构造包含 `80s`、`electronic`、`dance` 的 50 标签分数，断言它们全部出现在 `style`；构造 `Deep House` Discogs 输出，断言只进入 `genre`。在 `tests/w4dj_library.rs` 断言 `netease_genre` 不因两个新字段更新而改变。

- [x] **Step 2: 实现类型和序列化**

把旧的 MusiCNN 宽 Genre 投影保留为迁移辅助函数；新增 `style` 数组和 emotion/moodCluster 可选对象。Rust/TS DTO 使用 camelCase，SQLite 使用 `genre` 与 `style_tags_json` 分列。

- [x] **Step 3: 运行字段测试**

Run: `pnpm --dir app test -- --run app/src/analysis.test.ts`

Run: `cargo test --test w4dj_library`

Expected: 新字段通过，旧缓存读取不报错，网易云 Genre 保持不变。

### Task 2: MusiCNN Style 和三套后台情绪能力

**Files:**
- Modify: `app/src/analysis.ts`
- Modify: `app/src/app.ts`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/essentia_model_import.rs`
- Add (可选，真实歌曲评测通过后再随包启用): `src-tauri/resources/essentia-models` 中 TFJS 格式的情绪头资源；本阶段先完成注册、校验和测试夹具
- Test: `app/src/analysis.test.ts`

**Interfaces:**
- `style` 采用阈值筛选后的多标签，保存 `label`、`confidence` 和过滤原因。
- `emomusic-msd-musicnn` 和 `muse-msd-musicnn` 同时作为可选对照 head，分别写入 `emotionCandidates.emomusic` 与 `emotionCandidates.muse`，不在本阶段合并为单一 `emotion` 字段。
- `moods_mirex-msd-musicnn` 输出五个情绪簇到 `moodCluster`，不得写入 `emotionCandidates` 或 `mood`。
- 五个现有 Mood head 继续写入 `mood`，与三套新增结果并行但不混合。

- [x] **Step 1: 固定输出类型测试**

为 emoMusic 和 MuSe 各固定一组连续输出，验证两个模型都保留自己的模型名、`valence`、`arousal`，范围均为 1–9，且一个模型失败不会清空另一个模型的结果；为 MIREX 固定五个情绪簇输出，验证它只写入 `moodCluster`，不写入 `emotionCandidates` 或现有 `mood` 数组；同时验证 aggressive/happy/relaxed/party/sad 五个既有头仍可多标签同时成立。

- [x] **Step 2: 接入两个连续情绪对照头**

通过现有 200 维嵌入分别执行 emoMusic 和 MuSe；将结果写入两个独立候选字段，不平均、不覆盖。每个 head 单独记录缺失或失败原因；其中一个失败时保留另一个成功结果、基础分析、Style 和已有 Mood。

- [x] **Step 3: 接入 MIREX 情绪簇头**

通过现有 200 维嵌入执行 `moods_mirex-msd-musicnn`，校验输出恰好对应五个已知 MIREX 簇，将标签和置信度写入 `moodCluster`；不得复用连续坐标字段，也不得把簇标签塞进现有 Mood Tags。

- [x] **Step 4: 保留并并行运行现有五个 Mood 头**

继续加载 `mood_aggressive`、`mood_happy`、`mood_relaxed`、`mood_party`、`mood_sad`，把各自正类置信度作为独立 `mood` 标签保存。增加测试证明五个头可同时超过阈值，新三套情绪能力的缺失、取消和错误不会覆盖既有 Mood 结果。

- [x] **Step 5: 补充三套情绪模型导入校验**

为 `emomusic-msd-musicnn`、`muse-msd-musicnn` 和 `moods_mirex-msd-musicnn` 注册唯一 ID、输入宽度 200、输出维度（连续情绪为 2，MIREX 为 5）、输出层名称和类别列表；不满足结构或权重校验时拒绝安装。资源未安装时，状态只报告新增情绪字段缺失，不阻止基础分析、Style 或五个既有 Mood 头。

- [x] **Step 6: 运行 Worker/模型专项测试**

Run: `pnpm --dir app test -- --run app/src/analysis.test.ts app/src/analysis-worker-client.test.ts`

Run: `cargo test --manifest-path src-tauri/Cargo.toml essentia_model_import`

Expected: 旧 jobId 被忽略，取消不写当前歌曲，emoMusic/MuSe/MIREX/五个 Mood Tags 的字段不互相污染；没有新增默认 Dashboard 情绪列。

### Task 3: Discogs-EffNet Genre pipeline

**Files:**
- Modify: `app/src/analysis.ts`
- Modify: `app/src/analysis-worker-protocol.ts`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/essentia_model_import.rs`
- Modify: `scripts/prepare_essentia_tfjs_resources.py`
- Add: `src-tauri/resources/essentia-models` 中 Discogs-EffNet 和 Genre Discogs400 TFJS 资源
- Test: `app/src/analysis.test.ts`
- Test: `src-tauri/src/essentia_model_import.rs`

**Interfaces:**
- `EssentiaModelSpec` 增加 `embeddingFamily`、`inputShape`、`embeddingDimensions`、`outputName`、`task`。
- MusiCNN 路径保持 `[patch, 96] -> 200`；Discogs-EffNet 路径按官方 `[64, 128, 96] -> 1280` 形状执行。
- `genre` 保存 Discogs 主标签和候选标签；Discogs 不可用时 `genre` 为空，不回退为 `style`。

- [x] **Step 1: 写形状和资源拒绝测试**

加入错误输入形状、错误输出宽度、错误 outputName、manifest shard 截断和权重长度不匹配夹具；断言导入器拒绝这些文件并保留旧资源。

- [x] **Step 2: 实现模型族和输入准备**

新增 Discogs 专用 mel 批处理和 1280 维张量路径；保持所有同步推理在 Worker 中，不把 Discogs 的 1280 维张量送入现有 `[N, 200]` 分类头。

- [x] **Step 3: 转换并安装官方资源**

将官方模型转换为 `model.json + .bin`，重新读取 staged pair 后验证 graph、manifest、输入输出形状、权重长度和已知输出层，再通过 Tauri resources 离线安装。

- [x] **Step 4: 运行 Discogs 资源测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml essentia_model_import`

Run: `pnpm --dir app test -- --run app/src/analysis.test.ts`

Expected: 400 类风格输出正确进入 `genre`，MusiCNN 的 80s/情绪/属性标签仍进入 `style`；三套情绪候选和五个 Mood Tags 各自落入后台字段。

### Task 4: SQLite、Dashboard 和兼容迁移

**Files:**
- Modify: `src/w4dj_library.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `app/src/library-dashboard.ts`
- Modify: `app/src/library-dashboard.test.ts`
- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`

- [ ] **Step 1: 写投影和迁移测试**

旧记录的 `essentia_genre` 迁移到 Style；新记录同时保存 `netease_genre`、Discogs Genre 和 Style JSON；Genre/Style 查询、排序、分页和空值显示互不影响。

- [ ] **Step 2: 实现 Dashboard 双列和详情，暂不公开情绪字段**

默认列显示 `网易云 Genre`、`Genre`、`Style`；Genre 显示最高置信度主标签，详情显示候选；Style 显示多个标签及置信度。三套新增情绪结果和五个 Mood Tags 先保存在后台，不新增默认列、筛选项或音频标签回写；模型未完成时分别显示缺失，不显示 `null` 或伪造值。

- [ ] **Step 3: 验证旧缓存和取消行为**

确认旧 `track-analysis.json` 可读取，取消/失败保留旧投影，成功分析按 `destination_path` 更新同一输出记录，不创建转换历史。

- [ ] **Step 4: 运行完整回归**

Run: `pnpm --dir app test -- --run`

Run: `cargo test --all`

Run: `cargo fmt --all -- --check`

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Run: `git diff --check`

Expected: 现有测试全部通过，新增 Genre/Style/Emotion 覆盖通过；使用真实歌曲人工核对 `80s` Style 和 `Deep House` Genre。

## Acceptance Criteria

- 一首歌可以同时展示 `Genre: Deep House` 和 `Style: 80s · Electronic · Dance`。
- 网易云 Genre 不被两个模型字段覆盖。
- Style 不再因为宽 Genre 投影而丢失 80s 等非流派标签。
- Genre 缺失时不会用 Style 冒充；模型失败时旧结果和基础分析保留。
- emoMusic、MuSe 两组连续坐标和 MIREX 五簇全部可在后台独立测试；真实歌曲评测后再选择对外公开的连续 Emotion、MIREX 或 Mood Tags，三者不互相替代。
- Discogs-EffNet 资源在 Worker 离线加载，输入输出形状和权重通过严格校验。
- Dashboard、SQLite、兼容 JSON、转换写回和报告字段命名一致。
- 不修改版本号，不 commit、push、merge 或 release。
