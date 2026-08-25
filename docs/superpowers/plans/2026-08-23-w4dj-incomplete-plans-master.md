# W4DJ 3.2.0 beta-3 未完成计划统筹元计划

> 本文件只统筹当前工作树中已经存在的计划，不新增产品范围；不修改版本号，不提交、推送、合并或发布。

## 目标

把“代码实现完成”“自动化测试完成”和“真实数据/人工验收完成”分开记录，按依赖顺序收敛未完成事项。所有阶段完成后必须更新 `计划.md`、`docs/project-state.md` 和 `docs/handoff.md`，并提供最新 Apple Silicon App 产物。

## 当前清单

| 计划 | 当前状态 | 主要缺口 |
| --- | --- | --- |
| 文件名与冲突安全 | 代码与聚焦自动化已完成 | 真实 NetEase 样本、跨平台文件系统和手动报告验收 |
| 手动网易云元数据数据库 | 代码与自动化复核完成 | 真实数据库、FLAC/MP3 后台写回和 ExifTool 验收 |
| FLAC 封面数据库恢复 | 代码与自动化已接入，真实验收受限 | 89 首真实 FLAC、可匹配数据库、ExifTool、WAL/SHM 和后台批次验收 |
| 断点恢复与性能 | 代码与自动化完成 | 真实连续分析、Reload/取消/恢复和性能对比 |
| Discogs-EffNet 五 Head | 代码与资源自动化完成 | 真实重分析、写回复读和跨格式验收 |
| Energy 十级校准 | 代码与自动化完成 | 隐藏 WebView/jsdom 的阈值、ARIA、筛选、排序验收 |
| 情绪模型/盲听工具 | 代码与自动化完成 | 真实歌曲重分析、浏览器盲听、100/200 首评测 |
| Task 11/13 外部验收 | 本机自动化/构建完成 | MP3/FLAC/WAV/AIFF、Rekordbox、Windows、DMG；W4DJ GUI 不再作为验收入口 |
| Genre/Style 情绪计划 | 实现记录过期 | 只需同步后续 Discogs 已完成状态 |

## 执行顺序

1. 阶段 0：统一状态记录，禁止把自动化通过写成真实验收通过。（已完成）
2. 阶段 1：完成文件名与冲突安全；它是所有真实转换验收的前置条件。（代码与聚焦自动化已完成）
3. 阶段 2：复核手动网易云数据库入口，确保选择、清除和批次 resolver 规则一致。（代码与自动化复核已完成）
4. 阶段 3：在同一真实数据库和 89 首 FLAC 可用时完成封面恢复验收。（当前受环境素材限制）
5. 阶段 4：依次完成断点恢复/性能、Discogs、Energy、情绪模型真实验收。（本机代码/自动化已完成，真实歌曲与人工项目待补）
6. 阶段 5：完成 Task 11/13 外部环境验收、全量验证和 App 构建。（本机全量验证与 App 已完成，外部环境待补）

## 共同验收规则

- 所有数据库操作保持 read-only；不下载远程封面，不覆盖已有可靠标签或已有封面。
- 失败、超时、取消和缺少真实素材必须逐项记录，不能伪造通过。
- 不新增 hash、baseline、冻结 contract 或 release gate。
- 后续统一执行 `docs/superpowers/plans/2026-08-24-headless-acceptance.md`：不得打开 W4DJ App GUI，也不得依赖可访问性按钮；分析、转换和数据回读均使用隐藏运行时或 CLI 场景。
- 真实 89 首素材、Windows 或 Rekordbox 缺失时，先完成独立后台验收，再保留环境限制。主观盲听和 Rekordbox 实机导入单独列为外部人工项目，不能伪造为自动通过。
- 最终报告必须包括测试结果、人工验收结果、未执行项、`git status`、`git diff --stat` 和最新 App 链接。

## 阶段 2/3 现场核对记录

手动数据库入口的代码和自动化已复核通过。现场可读的迁移快照
`/Users/mac2/Music/网易云迁移存档/runs/20260823-173229-b737f8e2/before.sqlite3`
包含受支持表但 `track` 行数为 0，且没有对应的 `sqlite_storage.sqlite3` 或 89 首
FLAC 批次，因此不能执行真实匹配、封面写回或完整 ExifTool/WAL/SHM 前后对照。
现有 `Pinch - Qawwali.flac` 仅做只读 ExifTool 核对（3:48、23 MB、无可读标签）；
已有 `Qawwali - Pinch.mp3` 读到标题/艺人/专辑和封面。未修改这些文件。

阶段 4/5 本机结果：前端 jsdom 164/164、根 workspace `cargo test --all`、Tauri
51/51、Vite 构建、Tauri check/Clippy、Rust 格式和 diff-check 均通过。现有
`w4dj.sqlite3` 的 9 条分析记录为 6 completed、3 failed，尚未重新分析真实输出，
因此 genre/discogsEffnet/style 为空属于未执行真实推理，不是模型资源缺失。

## 2026-08-24 模型运行链复核补充

本轮补齐了模型 Worker 的二进制权重传输和旧 Mood/Voice 输出节点两处真实缺陷。30 秒真实 MP3 临时片段的完整 17 模型 Chromium Worker 烟测已完成：基础分析、Discogs Genre、五个 Discogs head、五个旧 Mood/Voice、emoMusic、MuSe、MIREX 均产生终态。该烟测只读临时 PCM，不等价于 9 首实际输出的回写验收；七项计划的真实长音频、数据库/ExifTool、GUI 和外部平台缺口继续保持未完成。

## 2026-08-24 无 GUI 验收迁移

后续真实验收统一使用 `docs/superpowers/plans/2026-08-24-headless-acceptance.md` 的隐藏 WebView/JSONL 入口；不再使用 GUI、可访问性按钮或截图触发。首轮 16 首整库验收因 WebContent 在首曲 MusiCNN 提取末段反复重启而阻断，未改写数据库或音频。
