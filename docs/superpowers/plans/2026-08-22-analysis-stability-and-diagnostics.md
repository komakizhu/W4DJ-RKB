# 增强分析稳定性与手动诊断报告实施记录

本文件记录 2026-08-22 稳定性与诊断修复的执行状态。实现不改变 SemVer、Task 6–13 的既有接口含义，也不自动生成或覆盖用户报告；用户仍需在转换历史中手动选择保存位置导出错误报告或运行会话记录。

## 任务清单

- [x] **Task 1：替换泄漏型逐帧提取**
  - `computeMusiCnnMelRows` 使用显式 `FrameGenerator(512, 256)`，逐帧复制 Mel 数组并释放 frame/vector；每 32 帧让出 Worker 事件循环并发送计数。
  - 保留 `patchSize=187`、`melBands=96`、尾部补零和 `tf.tensor3d` 输入形状；Tensor/模型在 `finally` 中释放。
- [x] **Task 2：Worker 心跳、超时和单首资源生命周期**
  - 新增固定超时公式模块、可选帧级进度、旧 `jobId/requestId` 过滤和幂等 `terminate()`。
  - 模型启动等待上限 120 秒；单曲超时为 `min(15 分钟, max(5 分钟, 时长秒数×3+60秒))`，错误带阶段、耗时和歌曲路径。
  - PCM 与模型权重使用可转移 `ArrayBuffer`，连续缓冲不再额外复制。
- [x] **Task 3：逐曲 Worker、超时继续和取消**
  - 每首未缓存歌曲创建、启动并销毁独立 Worker；超时记录 `status=timeout` 后继续下一首，普通错误记录 `failed`，取消不回写当前歌曲。
  - 运行会话记录 Worker ID、阶段、计数、耗时和终止原因；分析进度只更新局部 DOM，终态清除 `slotIndex`/Worker 状态。
- [x] **Task 4：分析状态与转换历史分离**
  - Rust 从运行会话候选、事件和分析报告推导 `AnalysisSessionSummary`，前端历史卡片分开显示转换与增强分析统计。
  - “导出错误报告”和“导出运行会话记录”均先打开保存对话框，取消保存不调用后端；分析不自动生成报告文件。
- [x] **Task 5：完整手动错误报告与元数据诊断**
  - 手动错误报告加入转换状态、增强分析总览/逐曲状态、增强分析报告和运行日志，覆盖 pending/running/completed/failed/timeout/cancelled；逐曲记录包含开始/结束时间、Worker、阶段、耗时和终止原因。
  - 分析失败字段增加状态、阶段、耗时；删除分析回写后的自动报告重写。
  - 元数据诊断区分可靠源标签与文件名推断，并独立报告“输出标签实际匹配”，避免文件名顺序造成误报。
- [x] **Task 6：文档、构建与真实验收**
  - 自动化验证、最终计数、环境限制和真实长音频/ExifTool/GUI 未执行项已写入 `计划.md`、`docs/project-state.md` 和 `docs/handoff.md`。

## 验收记录

- 前端：直接使用仓库内 Node/Vitest（`--configLoader runner`）运行全部 6 个测试文件，143/143 通过；由于沙箱禁止 Vite/Vitest 写入 `app/node_modules/.vite-temp`，未使用 pnpm 包装命令作为证据。
- Rust：使用 `CARGO_TARGET_DIR=/private/tmp/w4dj-stability-target` 避免沙箱目标目录写入限制；历史测试、Tauri 测试和 workspace 全量测试均通过。
- 构建/检查：Vite 输出到 `/private/tmp/w4dj-app-dist`；Rust fmt、check 和 Tauri all-targets Clippy 通过。真实 9 首长音频、ExifTool 标签、Windows/Rekordbox 和最终 GUI 操作仍需在相应环境人工验收。
