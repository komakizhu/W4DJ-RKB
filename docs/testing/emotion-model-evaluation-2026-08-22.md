# 情绪模型验收记录（2026-08-22）

## 自动准备结果（2026-08-22 历史快照）

本次使用当前应用数据目录中的 `w4dj.sqlite3` 做只读检查；为避免验收 CLI 的 schema 迁移写入用户数据库，先复制到 `/private/tmp/w4dj-emotion-db-copy.sqlite3`，再从副本生成 manifest。固定 seed 为 `20260822`。

- 5 首：`/private/tmp/w4dj-emotion-evaluation-5.json`，`sampleSize=5`。
- 20 首：`/private/tmp/w4dj-emotion-evaluation-20.json`，当前可用输出只有 10 首，因此 `sampleSize=10`。
- 100 首：`/private/tmp/w4dj-emotion-evaluation-100.json`，当前可用输出仍为 10 首，因此 `sampleSize=10`。
- 5 首 manifest 使用同一 seed 重复生成后，歌曲顺序一致；manifest 的 `sessionId` 按设计每次新建，不把整份文件 hash 当作验收条件。
- 当时三套新增 head 在资源目录中均不存在，所有样本的 `emomusic`、`muse`、`mirex` 状态均为 `model_missing`。这不是失败预测，也不进入任一模型的有效分母。
- 当前 10 条输出中只有 1 条存在已完成的 legacy Mood 结果；其余歌曲仍为未分析。该状态与 W4DJ SQLite/分析镜像一致。

当时资源目录只有 MusiCNN embedding、五个 legacy Mood head 和 voice/instrumental head；没有生成空 JSON、随机权重或运行时下载模型，也没有把缺失模型当作通过验收。

## 自动化工具验证

`tools/emotion-evaluation/evaluator.test.js` 与 `main.test.js` 共 8/8 通过。测试覆盖固定 seed 顺序、相对路径匹配、缺失模型排除、唯一/并列/无胜者计分、IndexedDB/本地恢复以及 JSON/CSV 手动导出。Rust manifest 集成测试通过，manifest 只读取 W4DJ 独立歌曲库的 `available` 输出，不枚举网易云 SQL，也不写回分析、历史或音频标签。

## 资源补齐后的当前状态（2026-08-23）

官方 `emomusic`、`muse`、`moods_mirex` head 已从 Essentia 官方 ONNX 导出离线转换为 TFJS `model.json + .bin`，并通过 Rust 导入器的输入/输出/权重长度校验后随 App 内置；没有占位权重，也没有运行时下载。Discogs EffNet 与 `genre_discogs400` 也已作为独立的正式 Genre pipeline 内置。旧 manifest 不会因安装模型而自动重写，因此其中已有的 `model_missing` 仍是当时分析快照；必须用当前 App 重新分析输出并重新生成 manifest，才能看到新的逐曲预测状态。

本机静态服务器已实际启动并通过 `curl http://127.0.0.1:1431/` 冒烟验证。当前 W4DJ 独立库只有 10 条实际可用输出，所以 20/100 请求仍分别得到 10/10 首；不能用重复歌曲填充 100/200 首。

## 尚未执行的人工验收

模型资源和 HTTP 服务阻塞已解除；仍不能由自动化替代浏览器播放、主观盲听和公开字段决策。这些步骤必须由用户在本机浏览器中执行，并且 100/200 首需要补充真实输出后才能执行。

资源准备完成后，人工步骤是：用同一 seed 生成 5、20 和全部可用（目标 100/200）manifest；在本地 HTTP 静态服务器打开 `tools/emotion-evaluation/index.html`；选择 manifest 和实际输出目录；每首只播放固定 10 秒片段，先记录主观情绪，再在匿名卡片中选择唯一、并列或都不符合；中途刷新确认 IndexedDB 恢复；最后手动导出 JSON/CSV。报告需分别记录每个模型的有效样本数、胜率/并列分数、按主观情绪分组、缺失/失败和不可播放数量，少于两套可用模型的歌曲不进入胜率分母。

在上述真实盲测完成前，Dashboard 不新增情绪默认列、筛选项、音频标签回写或公开设置；后台 JSON、SQLite analysis_results 和 manifest 状态字段保留，缺失 head 持续显示 `model_missing`。
