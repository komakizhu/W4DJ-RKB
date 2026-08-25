# Energy Ten-Level Calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `执行计划代理` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Do not dispatch subagents from a side conversation.

> **2026-08-24 验收入口更新：** 尚未完成的 Dashboard 人工 hover/筛选/排序验收改用 `2026-08-24-headless-acceptance.md` 规定的 jsdom/隐藏 WebView DOM、ARIA 和数值检查；不得打开 W4DJ App GUI。

**Goal:** 把 Dashboard 中现有的 Essentia RMS² Energy 原始数值转换为经过真实曲库校准的固定 1–10 级可见标度，同时完整保留原始 Energy 的存储、查询、排序和元数据行为。

**Architecture:** 新建纯前端 `energy-rating.ts`，只负责固定阈值到十级显示的无状态映射。`TrackAnalysis.energy`、`w4dj.sqlite3`、`track-analysis.json`、音频标签、筛选和排序继续使用原始 RMS²；Dashboard 表格和详情仅调用展示函数。校准来源、数学模型和复现步骤由 `docs/analysis/energy-rating-calibration-sop.md` 固化，运行时不根据用户曲库重新估计阈值。

**Tech Stack:** TypeScript、Vitest、现有 Vite/Tauri 前端、Essentia.js RMS² Energy、固定单调分段映射。

## Global Constraints

- Energy 的原始定义保持为 `Essentia.Energy(mono) / mono.length`，不得改动音频解码、44.1 kHz 重采样、mono 混合或 Worker 分工。
- 固定分界点为 `0.041982342`、`0.078189338`、`0.111694549`、`0.144505646`、`0.184125843`、`0.214877708`、`0.251289228`、`0.292838207`、`0.355045821`；等于分界点进入较高一级。
- 只修改 Dashboard 可见标度。SQLite、JSON、音频元数据、Rust DTO、查询字段和排序字段继续保存原始浮点 Energy。
- `null`、`NaN` 和无穷值显示 `—`；有限负值按 1 级显示；高于最高分界点按 10 级显示。
- Danceability 继续使用现有 `danceability-rating.ts` S 曲线，本计划不得合并或重算该指标。
- 不在运行时读取校准 CSV，不根据当前曲库动态分桶，不新增 hash、baseline、冻结 contract 或发布 gate。
- 不修改版本号，不提交、不 push、不 merge、不创建 Release；完成后展示测试、`git status` 与 diff 摘要，等待用户确认。
- 保留当前脏工作树；仅修改本计划列出的文件，不清理未跟踪文件。

---

## File Structure

**Create:**

- `app/src/energy-rating.ts`：固定 Energy 分界点、十级映射和可见格式化。
- `app/src/energy-rating.test.ts`：固定锚点、边界语义、缺失值和单调性回归。

**Modify:**

- `app/src/library-dashboard.ts`：表格和详情使用 Energy 十级显示，tooltip 保留原始数值。
- `app/src/library-dashboard.test.ts`：验证十级显示和原始筛选字段仍存在。
- `计划.md`：实施完成后勾选 Energy 补充任务。
- `docs/project-state.md`：实施完成后记录可见标度与原始数据边界。
- `docs/handoff.md`：实施完成后记录接口、测试和未执行人工验收。

**Reference only:**

- `docs/analysis/energy-rating-calibration-sop.md`
- `docs/analysis/energy-calibration-reference.py`
- `docs/analysis/data/energy-calibration-ratings.csv`
- `docs/analysis/danceability-rating-sop.md`

### Task 1: 固定 Energy 十级领域函数

**Files:**

- Create: `app/src/energy-rating.test.ts`
- Create: `app/src/energy-rating.ts`

**Interfaces:**

- Consumes: `number | null`，数值为现有 Essentia RMS² Energy 原始值。
- Produces: `ENERGY_LEVEL_THRESHOLDS`、`energyLevel(value)`、`formatEnergyRating(value)`。

- [x] **Step 1: 写失败的边界与格式测试**

创建 `app/src/energy-rating.test.ts`：

```ts
import { describe, expect, it } from 'vitest';
import {
  ENERGY_LEVEL_THRESHOLDS,
  energyLevel,
  formatEnergyRating,
} from './energy-rating';

describe('energy ten-level rating', () => {
  it('maps representative RMS-squared values to the approved levels', () => {
    expect(energyLevel(0.041)).toBe(1);
    expect(energyLevel(0.05)).toBe(2);
    expect(energyLevel(0.10)).toBe(3);
    expect(energyLevel(0.13)).toBe(4);
    expect(energyLevel(0.16)).toBe(5);
    expect(energyLevel(0.20)).toBe(6);
    expect(energyLevel(0.23)).toBe(7);
    expect(energyLevel(0.27)).toBe(8);
    expect(energyLevel(0.32)).toBe(9);
    expect(energyLevel(0.40)).toBe(10);
  });

  it('puts an exact threshold into the higher level', () => {
    ENERGY_LEVEL_THRESHOLDS.forEach((threshold, index) => {
      expect(energyLevel(threshold * (1 - 1e-9))).toBe(index + 1);
      expect(energyLevel(threshold)).toBe(index + 2);
    });
  });

  it('handles missing and finite extreme values', () => {
    expect(energyLevel(null)).toBeNull();
    expect(energyLevel(Number.NaN)).toBeNull();
    expect(energyLevel(Number.POSITIVE_INFINITY)).toBeNull();
    expect(energyLevel(-1)).toBe(1);
    expect(energyLevel(100)).toBe(10);
  });

  it('formats the approved six-of-ten example', () => {
    expect(formatEnergyRating(0.208673633)).toBe('★★★ 6/10');
    expect(formatEnergyRating(null)).toBe('—');
  });
});
```

- [x] **Step 2: 运行定向测试并确认失败**

Run:

```bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run src/energy-rating.test.ts
```

Expected: FAIL，原因是 `./energy-rating` 尚不存在。

- [x] **Step 3: 实现最小纯函数模块**

创建 `app/src/energy-rating.ts`：

```ts
export const ENERGY_LEVEL_THRESHOLDS = [
  0.041982342,
  0.078189338,
  0.111694549,
  0.144505646,
  0.184125843,
  0.214877708,
  0.251289228,
  0.292838207,
  0.355045821,
] as const;

export function energyLevel(value: number | null): number | null {
  if (value == null || !Number.isFinite(value)) return null;
  const safeValue = Math.max(0, value);
  const boundaryIndex = ENERGY_LEVEL_THRESHOLDS.findIndex(
    (threshold) => safeValue < threshold,
  );
  return boundaryIndex === -1 ? 10 : boundaryIndex + 1;
}

export function formatEnergyRating(value: number | null): string {
  const level = energyLevel(value);
  if (level == null) return '—';
  const solidStars = Math.floor(level / 2);
  const outlineStar = level % 2 === 1 ? '☆' : '';
  return `${'★'.repeat(solidStars)}${outlineStar} ${level}/10`;
}
```

- [x] **Step 4: 运行定向测试并确认通过**

Run the Step 2 command.

Expected: `energy-rating.test.ts` 全部通过。

### Task 2: Dashboard 可见标度接入

**Files:**

- Modify: `app/src/library-dashboard.test.ts`
- Modify: `app/src/library-dashboard.ts`

**Interfaces:**

- Consumes: Task 1 `formatEnergyRating(value)`。
- Produces: 表格与详情的 Energy 十级 HTML；原始数值只出现在 tooltip 和既有查询对象中。

- [x] **Step 1: 写失败的 Dashboard 回归测试**

在 `library-dashboard.test.ts` 导入 `formatEnergyRating`，并把现有格式测试扩展为：

```ts
expect(formatEnergyRating(0.8)).toBe('★★★★★ 10/10');
expect(formatEnergyRating(null)).toBe('—');
```

在事实表测试中增加：

```ts
expect(html).toContain('★★★★★ 10/10');
expect(html).toContain('Essentia RMS² raw: 0.8');
expect(html).toContain('value="energy"');
```

保留已有 `formatDanceabilityRating` 断言，证明两个标度互不覆盖。

- [x] **Step 2: 运行 Dashboard 测试并确认失败**

Run:

```bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run src/library-dashboard.test.ts
```

Expected: FAIL，Dashboard 仍显示原始 `0.80`。

- [x] **Step 3: 接入 Energy formatter**

在 `library-dashboard.ts` 顶部增加：

```ts
import { formatEnergyRating } from './energy-rating';
```

在 `renderDanceabilityRating` 邻近位置增加：

```ts
function renderEnergyRating(value: number | null): string {
  const text = formatEnergyRating(value);
  const [stars, rating = ''] = text.split(' ');
  const rawValue = value == null || !Number.isFinite(value) ? '—' : String(value);
  const title = `Essentia RMS² raw: ${rawValue} · ${text}`;
  return `<span class="library-rating" title="${escapeHtml(title)}"><span class="library-rating-stars">${escapeHtml(stars)}</span>${rating ? `<span class="library-rating-percent">${escapeHtml(rating)}</span>` : ''}</span>`;
}
```

把表格分支改为：

```ts
case 'energy': return `<td>${renderEnergyRating(track.energy)}</td>`;
```

把详情中的 `LUFS / Energy` 改为：

```ts
<div><dt>LUFS / Energy</dt><dd>${formatNumber(track.integratedLoudnessLufs, ' LUFS')} / ${renderEnergyRating(track.energy)}</dd></div>
```

不要修改 `LibraryTrack.energy`、filter field、sort field、DTO 或 SQLite 查询。

- [x] **Step 4: 运行 Energy、Dashboard 与完整前端测试**

Run:

```bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run src/energy-rating.test.ts src/library-dashboard.test.ts
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run
```

Expected: 所有前端测试通过；既有 Danceability、双槽、Worker 和 Dashboard 测试不回归。

### Task 3: 数据边界、文档与构建验收

**Files:**

- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`

**Interfaces:**

- Consumes: Tasks 1–2 的固定展示函数和测试结果。
- Produces: 主规划完成状态、项目状态与后续会话可恢复的交接说明。

- [x] **Step 1: 核对原始 Energy 数据链未改变**

Run:

```bash
git diff -- app/src/analysis.ts src/analysis.rs src/w4dj_library.rs src/library_query.rs src/metadata.rs src/sync.rs
```

Expected: 本任务没有修改这些文件。若出现 diff，只能是实施前已有用户改动，必须在交付中明确归属，不得覆盖。

- [x] **Step 2: 更新计划和状态文档**

把 `计划.md` 的 Energy 补充任务勾选为完成；在 `docs/project-state.md` 和 `docs/handoff.md` 中写明：

```text
Energy Dashboard 展示采用固定十级阈值，只转换可见等级；原始 Essentia RMS² 值继续用于 SQLite/JSON、查询、排序和音频标签。49 首人工锚点与 1245 首有效分布的复现方法见 docs/analysis/energy-rating-calibration-sop.md。
```

测试数量必须按本次真实输出填写，不预先写死。

- [x] **Step 3: 运行完整验证**

Run:

```bash
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app build
cargo test --all
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --lib --all-features -- -D warnings
cargo clippy --all-targets --all-features -- -A dead_code -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -A dead_code -D warnings
git diff --check
```

Expected: 全部通过；若 pnpm wrapper 仍受 `ignored build scripts` 安全策略影响，使用仓库现有 Node/Vitest/Vite 直接运行方式并如实记录，不把未运行项写成通过。

- [ ] **Step 4: 人工核对 Dashboard**

打开歌曲库，确认 Energy 表格与详情显示十级星级；hover 能看到原始 RMS²；按 Energy 排序和原始数值筛选的顺序与实施前一致；Danceability 显示不变。抽查阈值两侧各一首，并确认等于阈值进入较高一级。

- [x] **Step 5: 展示工作树并停止**

Run:

```bash
git status --short
git diff --stat
```

Expected: 展示本任务文件和所有既有脏工作树改动；不提交、不 push，等待用户确认。
