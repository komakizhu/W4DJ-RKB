# W4DJ Danceability 十级展示 SOP

## 当前状态

Danceability 十级展示已经实现于 `app/src/danceability-rating.ts`。它只改变 Dashboard 可见等级，不改变 Essentia 原始 Danceability、SQLite/JSON、查询、排序或音频标签。

当前曲线来自既有人工锚点，不是 Energy 的 49 首动态规划结果。两种指标必须分别维护，不能复用阈值。

## 原始指标链

实现位置：`app/src/analysis.ts`。

应用使用与 Energy 相同的 44.1 kHz mono vector 调用：

```text
danceability_raw = Essentia.Danceability(mono).danceability
```

只接受有限数值；分析结果原样保存。Dashboard 查询和排序使用 `danceability_raw`，十级函数只负责显示。

## 固定 S 曲线

当前常量：

```text
slope = 4.48056
midpoint = 1.10370
```

连续值：

```text
S(x) = 1 + 9 / (1 + exp(-4.48056 × (x - 1.10370)))
```

显示等级：

```text
level = clamp(round(S(x)), 1, 10)
```

`null`、`NaN` 和无穷值显示 `—`。有限极端值限制到 1–10。

## 反解后的等级边界

对 `S(x)=1.5, 2.5, ..., 9.5` 反解：

```text
x = midpoint + ln((S-1)/(10-S)) / slope
```

得到：

| 等级 | Essentia Danceability 原始值 |
|---:|---:|
| 1 | `< 0.4713653490` |
| 2 | `0.4713653490 ≤ x < 0.7444953666` |
| 3 | `0.7444953666 ≤ x < 0.8904428524` |
| 4 | `0.8904428524 ≤ x < 1.0028230731` |
| 5 | `1.0028230731 ≤ x < 1.1037000000` |
| 6 | `1.1037000000 ≤ x < 1.2045769269` |
| 7 | `1.2045769269 ≤ x < 1.3169571476` |
| 8 | `1.3169571476 ≤ x < 1.4629046334` |
| 9 | `1.4629046334 ≤ x < 1.7360346510` |
| 10 | `x ≥ 1.7360346510` |

JavaScript `Math.round` 对正数半值向上，因此等于边界进入较高一级。

## 已固定锚点

现有测试 `app/src/danceability-rating.test.ts` 固定：

```text
0.8240978122 → 3
1.1535       → 6
2.8114326    → 10
```

其中 Joe Fight 的约 `1.1535 → 6` 已作为实际显示锚点；Friday Night 的 `2.8114326 → 10` 是既有校准锚点，但当前真实曲库中未完成同等外部验收。因此文档不得宣称整个 Danceability 分布已像 Energy 一样通过 49 首盲评和边界组删除检验。

## 复现与测试

运行：

```bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run src/danceability-rating.test.ts src/library-dashboard.test.ts
```

测试必须覆盖：

- 三个固定锚点。
- `null`、NaN、无穷值和有限极端值。
- 观察范围内单调不下降。
- Dashboard tooltip 保留 Essentia 原始值。
- 原始查询和排序字段仍为 `danceability`，不改成可见等级。

人工复验时，先导出原始 `danceability`，再记录盲评。若未来需要像 Energy 一样重新校准，应建立独立评分 CSV、覆盖十分位并在候选边界附近主动选样；不得把 Energy 的目标占比、Huber 参数或最终阈值直接复制到 Danceability。

## 重新校准条件

- Essentia Danceability 算法、版本、输入声道或采样率发生变化。
- 获得足够的独立盲评数据并明确要求重新定义十级语义。
- 真实曲库显示长期集中在少数档位，且已排除分析失败和样本缺失。

普通新增歌曲或单个主观分歧不触发重新校准。
