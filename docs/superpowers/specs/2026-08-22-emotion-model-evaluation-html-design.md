# 情绪模型主观验收 HTML 工具设计

## 目标

为 W4DJ 提供一个与正式 Dashboard 分离的工作区 HTML 验收工具，用同一批真实歌曲、同一个 10 秒片段和随机匿名呈现，比较四套情绪系统：现有五个 Mood 头组成的旧基线、emoMusic、MuSe 和 MIREX。工具收集用户的主观听感选择，输出可复核的 JSON/CSV 报告，帮助决定后续公开和内置哪套情绪能力。

## 已确认的产品决策

- 验收工具放在工作区，不集成到 W4DJ Dashboard 或正式设置界面。
- 工具使用独立 HTML 页面，通过本地地址运行，不依赖 CDN，不上传音频或结果。
- 每首歌播放固定 10 秒；片段由统一算法自动选择最高能量的 10 秒。如果歌曲已有有效 Drop 窗口，则优先在该窗口内选择最高能量的 10 秒；没有有效 Drop 窗口时在全曲滑动搜索；歌曲短于 10 秒时使用整首并标记为短片段。四套系统必须使用完全相同的片段。
- 歌曲顺序和四套模型卡片顺序随机；随机种子、歌曲顺序和每首歌的卡片顺序全部保存，以便复现。
- 四个比较对象是“系统”而不是单个模型文件：
  1. 旧 Mood 基线：`mood_aggressive`、`mood_happy`、`mood_relaxed`、`mood_party`、`mood_sad` 五个头合并为一张卡片，五个头的原始分数仍单独保留。
  2. `emomusic-msd-musicnn`。
  3. `muse-msd-musicnn`。
  4. `moods_mirex-msd-musicnn`。
- 人工主观标签使用独立于模型的六个选项：明亮、悲伤、平静、激烈、兴奋、中性/其他；另有“无法判断”。主观标签只作为背景和分组统计，不直接决定模型胜负。
- 每首歌先在不显示模型结果的情况下选择主观标签，再显示匿名 A/B/C/D 卡片，选择最符合听感的一套；允许选择并列或“都不符合”。
- HTML 读取 W4DJ 只读导出的 `emotion-evaluation-manifest.json`，不在浏览器中重复运行模型。
- HTML 启动时由用户选择输出音频文件夹，使用 manifest 中相对于输出根目录的 `relativePath` 匹配文件；找不到的歌曲标记为缺失，不计入胜率。
- 每完成一首立即保存本地会话进度；验收结果写入独立存储，不修改 `w4dj.sqlite3`、`track-analysis.json`、音频文件或转换历史。

## 组件边界

### W4DJ manifest 导出

新增只读的 `export_emotion_evaluation_manifest` 能力，从 `<app-data>/w4dj.sqlite3` 选择 `available` 歌曲，接收歌曲数量和随机种子，生成 manifest。它读取同一输出记录绑定的四套预测结果和旧五头的分头结果，按 `peak-energy-10s-with-drop-preference` 片段策略写入 `clipStartSeconds` 和 `clipDurationSeconds`。该能力不触发重新分析、不改变歌曲库状态、不创建转换历史。

Manifest 至少包含以下结构：

```json
{
  "schemaVersion": 1,
  "sessionId": "emotion-eval-2026-08-22-001",
  "seed": 12345,
  "sampleSize": 100,
  "clipPolicy": "peak-energy-10s-with-drop-preference",
  "tracks": [
    {
      "trackId": "w4dj-track-id",
      "title": "Example",
      "artist": "Artist",
      "relativePath": "Album/01 Example.wav",
      "durationSeconds": 214.4,
      "clipStartSeconds": 42.0,
      "clipDurationSeconds": 10.0,
      "legacyMood": {
        "status": "completed",
        "labels": [{ "label": "happy", "confidence": 0.91 }],
        "heads": {
          "mood_happy": { "positive": 0.91, "negative": 0.09 }
        }
      },
      "emomusic": {
        "status": "completed",
        "valence": 7.1,
        "arousal": 6.4
      },
      "muse": {
        "status": "completed",
        "valence": 6.8,
        "arousal": 5.9
      },
      "mirex": {
        "status": "completed",
        "labels": [{ "label": "cluster_3", "confidence": 0.74 }]
      }
    }
  ]
}
```

模型结果必须带有 `completed`、`missing` 或 `failed` 状态。缺失或失败的卡片可以在页面中显示状态，但不能被计为模型胜利；四套系统都不可用的歌曲保留在报告中并标记为无效样本。

### 工作区 HTML 验收工具

工具放在 `tools/emotion-evaluation/`，至少包含 `index.html`、页面脚本和使用说明。页面不依赖正式 App 的 DOM 或状态，只消费 manifest 和用户选择的音频目录。

页面流程固定为：

1. 选择 manifest 文件。
2. 选择音频输出文件夹；按 `relativePath` 建立文件句柄索引。
3. 选择开始新一轮或恢复已有 session。
4. 显示当前歌曲标题、艺术家、10 秒播放控件和进度，不显示模型名称。
5. 用户先从“明亮、悲伤、平静、激烈、兴奋、中性/其他、无法判断”中选择主观听感。
6. 用户点击继续后，显示匿名 A/B/C/D 卡片。连续模型以“愉悦度 / 激烈度”显示，MIREX 显示其簇标签，旧基线显示 Mood 标签；卡片布局统一，真实模型名在页面中隐藏。
7. 用户选择 A/B/C/D 中最符合的一套，或选择“并列”/“都不符合”，然后提交本首。
8. 提交后立即保存并进入下一首；可以暂停、返回上一首修改，或关闭页面后恢复。
9. 完成后显示汇总并提供 JSON、CSV 导出；导出的明细包含匿名卡片到真实模型的映射，便于复核。

歌曲顺序使用带种子的无放回洗牌；每首歌的四张卡片使用独立的带种子洗牌。页面只使用 manifest 中已经保存的顺序，不能在恢复 session 时重新洗牌。

### 本地持久化和导出

会话数据使用浏览器 IndexedDB 保存，键包含 `sessionId`、manifest 版本和 trackId。每条记录至少保存：主观标签、四卡片的展示顺序、最终选择、提交时间、音频匹配状态和用户备注（可选）。

导出的 JSON 保留完整原始模型输出、人工选择和评分中间值；CSV 提供一行一首歌的平面字段，方便用表格或统计脚本复核。页面不自动写入工作区文件，用户通过明确的“导出 JSON / 导出 CSV”按钮下载结果。

## 随机与评分

四套系统的卡片顺序每首歌随机，页面不显示真实模型名。主指标是用户对“最符合听感”的选择：唯一胜者得 1 分；两套并列各得 0.5 分；三套并列各得 1/3 分；“都不符合”记录为失败样本但不进入胜率分母。音频缺失、模型缺失和模型失败独立统计，不伪装为主观失败。

报告至少给出：四套系统总分、胜率、有效样本数、每种主观标签下的胜率、并列比例、“都不符合”比例、缺失比例、100 首和 200 首批次对比，以及卡片位置与选择之间的交叉统计。如果第一名和第二名差距很小，报告必须显示“无法区分”，不能自动替用户决定正式模型。

旧 Mood 基线的五个头只合并为一个竞争卡片；报告另附五个头的分数和是否超过阈值，作为辅助诊断，不改变四系统主排名。

## 错误处理

- manifest schema 版本不支持、字段缺失或 JSON 损坏：阻止开始并指出具体字段。
- 音频目录中找不到 `relativePath`：该首显示“文件缺失”，允许跳过但保留记录。
- 音频无法解码或 10 秒片段越界：标记“不可播放”，不计入胜率。
- 某套模型状态为 `missing` 或 `failed`：卡片显示不可用，不允许选择为胜者。
- 浏览器刷新或关闭：从 IndexedDB 恢复最后一个未提交样本；已提交结果不可重复计分。
- 导出失败：保留 IndexedDB 会话，显示重试按钮，不清理结果。

## 测试和验收

自动化测试覆盖：带种子洗牌在相同输入下产生相同歌曲/卡片顺序；不同种子改变顺序；`relativePath` 可区分同名不同目录；缺失和失败状态不进入分母；唯一胜者、两方并列、三方并列和“都不符合”计分正确；IndexedDB 恢复不会重复提交；manifest 版本和模型状态校验拒绝坏输入；JSON/CSV 导出字段完整。

人工验收先用 5 首固定夹具验证完整流程，再用 20 首真实歌曲验证断点续测和文件匹配，最后运行 100 首或 200 首完整批次。人工验收结束后检查报告中四套系统的真实映射、随机种子、有效样本数和胜率分母，确认旧 Mood 基线没有被拆成五个竞争者，且 HTML 工具没有修改 W4DJ 歌曲库或正式分析缓存。

## 非目标

- 不把验收页面加入 Dashboard 默认导航或正式产品设置。
- 不在 HTML 中重复运行 Essentia、TensorFlow 或模型下载。
- 不用人工选择自动覆盖分析结果，不把验收胜者自动设为正式模型。
- 不修改版本号，不创建 baseline/hash/frozen contract，不 commit、push、merge 或发布。

## 实施记录（2026-08-22）

当前实现位于 `src/w4dj_library.rs`、`src-tauri/src/main.rs`、`src-tauri/src/bin/export_emotion_evaluation_manifest.rs` 和 `tools/emotion-evaluation/`。Manifest 只查询 W4DJ 独立歌曲库的 `available` 输出；片段选择顺序为 Drop、峰值能量、起点回退，并在 `clipSelection` 中保留来源。页面使用原生 ES modules，先盲听再展示匿名卡片，支持任意子集并列、IndexedDB/本地恢复和显式 JSON/CSV 导出。现有自动化覆盖 8 项纯逻辑/页面流程测试与 Rust manifest/编译验证；真实 emoMusic、MuSe、MIREX 结果写入和 5/20/100（或 200）首人工批次仍待后续模型接入及用户环境验收。
