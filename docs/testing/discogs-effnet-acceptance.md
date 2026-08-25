# Discogs-EffNet 验收记录

日期：2026-08-23（Asia/Shanghai）

## 已完成的自动化验收

- 前端直接 Vitest：7 个测试文件，146 项通过；覆盖 EffNet Mel 的 96×128 输入、尾部补零、共享 embedding、五个 head 的多标签/多分类聚合、独立失败、旧任务过滤和 Dashboard 展示/筛选。
- Tauri 单元测试：47 项通过；覆盖六组资源的输入输出合同、输出节点、类别/权重长度、恢复和导入安全性、状态 DTO、SQLite 投影与命名空间回写。
- 独立歌曲库集成测试：7 项通过；覆盖迁移、成功投影、失败/缺失 head 保留既有值。
- 手动错误报告测试覆盖五个 head 的 `completed`、`model_missing`、`failed`、`timeout` 和 `cancelled` 状态，且转换状态与增强分析状态分开。

## 仍需人工执行的真实验收

在有真实音频和模型资源的本机，使用 Dashboard 的“重新分析当前输出”处理现有输出目录，然后确认每首记录的 `highLevel.discogsEffnet.heads` 都包含五个独立状态；取消或单 head 失败不得清除其它 head 或既有元数据。

分析成功后，用 ExifTool 检查 `W4DJ-Discogs-MoodTheme`、`W4DJ-Discogs-Approachability`、`W4DJ-Discogs-Instrumentation`、`W4DJ-Discogs-Timbre` 和 `W4DJ-Discogs-Danceability`。同时确认原有 `W4DJ-Danceability`、标准 Genre、BPM 和 Key 未被覆盖。对 MP3、FLAC、WAV、AIFF 分别执行（环境有对应格式时）。

当前会话未执行上述真实重分析和 ExifTool 回读，也未执行 Windows、Rekordbox、浏览器盲听或 100/200 首人工评测；这些项目不能用离线自动化结果冒充完成。模型安装不会自动补写旧分析记录，必须重新分析后才能看到新增结果。
