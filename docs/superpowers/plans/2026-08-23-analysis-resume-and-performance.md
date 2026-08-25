# 增强分析断点恢复与性能优化实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `执行计划代理` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **2026-08-24 验收入口更新：** 未完成的连续分析、取消、恢复、关闭/重启和 ExifTool 验收改用 `2026-08-24-headless-acceptance.md` 的共享 runner 与隐藏运行时；不再用 reload、可访问性按钮或可见窗口触发。

**目标：** 解决 WebView reload、关闭或异常重建导致增强分析静默消失的问题，并减少 MusiCNN、Discogs-EffNet 的重复计算和内存分配。

**架构：** Rust 后端持久化分析批次、逐曲状态和心跳，前端 Worker 仍负责 Essentia/TensorFlow 推理。页面重载后重建 Worker 并从未完成歌曲继续；性能优化保持模型和数值语义不变。

> **当前执行记录（2026-08-23）：** 运行会话定位、原子逐曲状态、15 秒心跳接管、reload 恢复入口、单曲 Worker 生命周期、超时继续、MusiCNN 固定批次双输出和 Discogs 流式 embedding 已实现。前端 Vitest 147/147、Tauri 49/49、根 workspace `cargo test --all` 412 项、Vite、fmt、check 和 Tauri 严格 Clippy 已通过；真实长音频/WebView 后端对比、9 首连续 reload/关闭验收、ExifTool 回读和跨平台验收仍未执行。

**技术栈：** Rust/Tauri、TypeScript/Vite/Worker、Essentia.js WASM、TensorFlow.js、现有运行会话与 W4DJ 独立歌曲库。

## 全局约束

- 不修改版本号。
- 不修改或删除输出音频及已有成功分析。
- 已完成歌曲不得重复分析，除非用户明确选择强制重新分析。
- 不静默回退主线程推理。
- 不新增 baseline、hash 或发布 gate。
- 完成后编译最新 App，但不 commit、push、merge 或发布。

---

### Task 1：修复运行会话定位和错误历史状态

**文件：**

- 修改：`src/history.rs`
- 修改：`src-tauri/src/main.rs`
- 修改：`app/src/app.ts`
- 测试：`tests/history.rs`、`src-tauri/src/main.rs`、`app/src/app.test.ts`

**接口：**

- `HistoryEntry` 增加向后兼容的可选 `runtime_session_dir`。
- `resolve_runtime_session_dir(root, entry)` 成为历史加载、错误报告导出和运行会话导出的统一解析入口。

- [x] 添加回归测试：历史 ID 为 `batch-...-slot2`、`batch_id` 为 `batch-...`，真实会话目录应被找到。
- [x] 创建运行会话后，将真实目录写入对应历史记录。
- [x] 优先读取历史记录保存的目录；旧记录回退到按 `batch_id` 搜索。
- [x] 规范化并校验目录位于配置的 `W4DJ-runtime-sessions` 根目录内。
- [x] 有 `analysis_started` 事件但没有终态时，不得显示“未请求”。
- [x] 运行历史、导出错误报告和导出运行会话的定向测试。

**验收：** 当前真实批次显示“增强分析：运行中/已中断，1/9”，导出内容不再声称找不到会话。

---

### Task 2：持久化逐曲状态、心跳和单例租约

**文件：**

- 修改：`src-tauri/src/main.rs`
- 修改：`app/src/app.ts`
- 测试：`src-tauri/src/main.rs`、`app/src/app.test.ts`

**接口：**

```ts
type AnalysisCandidateRunState =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'timeout'
  | 'interrupted'
  | 'cancelled';

type AnalysisRunSnapshot = {
  batchId: string;
  attemptId: string;
  status: 'running' | 'completed' | 'partial' | 'cancelled' | 'interrupted';
  heartbeatAt?: string;
  candidates: Array<{
    sourcePath: string;
    destinationPath: string;
    state: AnalysisCandidateRunState;
    stage?: string;
    processed?: number;
    total?: number;
    workerJobId?: string;
    reason?: string;
  }>;
};
```

- [x] 在运行会话目录增加 `analysis-state.json`，通过同目录临时文件加 rename 原子替换。
- [x] 在 `analysis_candidate_started` 前把歌曲持久化为 `running`。
- [x] 进度事件每秒更新心跳、阶段、帧数和 Worker ID。
- [x] 只有结果成功写入 W4DJ 数据库后才能将歌曲标记为 `completed`。
- [x] Worker 失败、超时和取消分别写入明确终态。
- [x] 新增 `claim_analysis_run`，同一批次只能存在一个有效 `attemptId`。
- [x] 心跳超过 15 秒或新页面接管旧批次时，将旧 `running` 歌曲改为 `interrupted`。
- [x] App 完全关闭后，下次启动同样根据过期心跳恢复状态；重启后事件会懒加载持久化监视器继续写入。
- [x] 测试单例拒绝、心跳过期、原子状态更新和旧状态文件兼容。

**验收：** 页面消失不会留下永久 `running`，也不能并发启动两个分析批次。

---

### Task 3：实现 reload 后断点恢复

**文件：**

- 修改：`app/src/app.ts`
- 修改：`app/src/analysis-worker-client.ts`
- 测试：`app/src/app.test.ts`、`app/src/analysis-worker-client.test.ts`

- [x] App 启动时调用 `load_incomplete_analysis_run`。
- [x] 恢复同一批次时只重新排队 `pending/interrupted/failed/timeout` 歌曲。
- [x] `completed` 歌曲直接读取数据库结果，不重新创建 Worker。
- [x] 当前歌曲继续按“一首一个 Worker”执行和销毁。
- [x] 用户主动取消产生 `cancelled`，重开后不得自动恢复。
- [x] reload 前尝试记录 `analysis_renderer_unloading`，但正确恢复不得依赖该事件一定成功写入。
- [x] 分析期间屏蔽右键 Reload，并在关闭或重新加载前显示中断提示。
- [x] UI 增加“继续未完成分析”，显示完成、失败、中断和待处理数量。
- [x] 测试 reload 恢复、完成歌曲跳过、取消不恢复和旧 Worker 消息过滤。

**验收：** 第二首分析到一半时 reload；重新打开后第一首不重跑，第二首重新开始，最终完成剩余歌曲。

---

### Task 4：补齐阶段诊断和性能测量

**文件：**

- 修改：`app/src/analysis-worker-protocol.ts`
- 修改：`app/src/analysis-worker-client.ts`
- 修改：`app/src/analysis.ts`
- 修改：`src/history.rs`
- 测试：对应前端测试与 `tests/history.rs`

**接口：**

```ts
type AnalysisDetailedStage =
  | 'decoding'
  | 'analyzingBasic'
  | 'extractingMusiCnn'
  | 'runningMusiCnn'
  | 'extractingDiscogs'
  | 'runningDiscogsEmbedding'
  | 'runningDiscogsHeads'
  | 'runningEmotionHeads'
  | 'persisting';
```

- [x] 每次阶段切换记录开始时间、结束时间和耗时。
- [x] 记录 `tf.getBackend()`、patch 数量以及 `tf.memory()` 可用指标。
- [x] 报告显示最后心跳、最后阶段和各阶段耗时。
- [x] 超时错误精确到模型族、模型 ID 和阶段。
- [ ] 优化前用同一首 `I Feel Love` 完成三次暖运行计时，不保存独立 baseline 文件。

---

### Task 5：优化 MusiCNN 特征和推理

**文件：**

- 修改：`app/src/analysis.ts`
- 测试：`app/src/analysis.test.ts`

- [x] 将 MusiCNN 的 `number[][]` 特征存储替换为预分配 `Float32Array`（保留 `melRows` 兼容返回字段）。
- [x] 直接写入 `[patchCount, 187, 96]` 连续缓冲，并保持现有尾部补零规则。
- [x] 以固定 patch 批次推理，避免为整首歌一次创建大型嵌套数组。
- [x] 一次 `model.execute()` 同时取得 `model/dense/Relu` 和 `model/Sigmoid`，不再重复执行同一主网络。
- [x] 每批 Tensor 在读取结果后立即 `dispose()`。
- [ ] 固定输入比较优化前后的 embedding、tag 分数和标签结果，使用合理浮点容差且不新增 hash/baseline。
- [ ] 测试尾部补零、批次边界、双输出调用次数、资源释放和数值一致性。

**预期收益：** 减少对象分配、垃圾回收和重复神经网络计算。

---

### Task 6：优化 Discogs-EffNet 和 TensorFlow 后端

**文件：**

- 修改：`app/src/analysis.ts`
- 修改：`app/src/discogs-effnet.ts`
- 修改：`app/src/analysis.worker.ts`
- 测试：`app/src/analysis.test.ts`、Worker 测试

- [x] 将 Discogs 特征改为流式批次：生成一批、推理一批、释放一批。
- [x] 不再保存整首歌曲的全部 Discogs `number[][]`。
- [ ] MusiCNN 与 Discogs 共用一次 `FrameGenerator(512, 256)` 遍历，但仍分别执行各自的特征转换。
- [x] 保持 Discogs 输入 `[64, 128, 96]`、有效 patch 计数和五个 head 的独立状态不变。
- [ ] 在目标 macOS WebView Worker 中实测 CPU、WebGL 和可随包运行的 WASM/SIMD 后端。
- [ ] 只采用真实长音频上更快且输出稳定的后端；模型和运行库不得联网下载。
- [x] 暂不并行分析多首歌曲，避免提升 WebContent 内存峰值。

---

### Task 7：报告、自动化测试与真实验收

**文件：**

- 修改：`计划.md`
- 修改：`docs/project-state.md`
- 修改：`docs/handoff.md`

- [x] 报告完整覆盖所有候选歌曲，不再只列已完成歌曲。
- [x] 将无终态且心跳过期的歌曲显示为 `interrupted`。
- [x] 明确区分“转换 9/9”和“增强分析 1/9”。
- [x] 验证错误报告与运行会话导出均能找到真实会话目录。
- [x] 更新计划、项目状态和交接文档中的实际测试结果。
- [x] 运行完整自动化验证：

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

- [ ] 使用现有 9 首输出完成真实验收：正常完成、第二首中途 reload、关闭 App 后恢复、主动取消、重新继续。
- [ ] 确认已完成结果不重复计算或覆盖，最终报告包含 9 首完整终态。
- [ ] 对 `I Feel Love` 做三次暖运行，优化后中位耗时至少降低 30%。
- [ ] 使用 ExifTool 确认优化前后 BPM、Key、LUFS、Energy、Danceability 和 Genre 回写一致。
- [x] 编译最新 macOS App，并提供可点击的产物路径。
- [ ] 最后报告 `git status`、`git diff --stat`、测试结果和环境限制；不提交或推送。

## 完成标准

- reload、WebView 重建或 App 重启不再导致分析批次静默丢失。
- 历史界面和两个手动导出文件准确显示逐曲状态。
- 同一批次不会并发启动两个分析 Worker 队列。
- 9 首真实输出可从中断位置继续并最终得到完整终态。
- 分析期间 UI 保持可操作，成功结果和音频标签不因中断被覆盖。
- 性能优化不改变模型输入形状、标签语义或已有分析接口。

## 2026-08-24 验收入口迁移

真实长音频/取消恢复验收改用隐藏 `libraryAnalysis` runner 和 JSONL/SQLite/ExifTool 证据，不再依赖 Dashboard GUI 或截图；首轮 16 首运行因 WebContent 重启阻断，未将恢复验收标记为通过。
