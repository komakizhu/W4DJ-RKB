# 增强分析进度槽位路由修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让增强分析进度始终显示在触发该批次的任务槽中，任务 2 的分析不再错误覆盖任务 1 的进度条。

**Architecture:** 当前增强分析仍由一个前端批次统一编排，但在进入分析前按 `AppPreview.slot_index` 分组。每组分析使用独立的 `AppAnalysisState` 计数，并携带 `slotIndex`；渲染层和增量 DOM 更新只更新匹配槽位。转换、Worker 消息协议、取消和后端接口保持不变。

**Tech Stack:** TypeScript、Vite、Vitest、现有 `app/src/app.ts` 渲染/分析编排。

## Global Constraints

- 不修改 Rust 转换、Tauri 命令、Worker 消息协议或版本号。
- 不改变增强分析的数值链、缓存键、取消语义和 `applyTrackAnalysisResults` 参数。
- 进度事件只更新对应任务卡，不重建整棵 DOM；完成、取消、错误仍使用现有低频完整渲染。
- 不提交、push、merge 或发布；保留共享工作树中的其他修改。

---

### Task 1: 建立任务槽感知的分析状态与候选分组

**Files:**
- Modify: `/Users/mac2/Documents/W4DJ RKB/app/src/app.ts:211-221,916-926,3008-3218`
- Test: `/Users/mac2/Documents/W4DJ RKB/app/src/app.test.ts`

**Interfaces:**
- Consumes: `AppPreview.slot_index`、`AppPreview.preview.candidates`、现有 `analyzeAudioFile`。
- Produces: `AppAnalysisState.slotIndex` 和按 `slot_index` 分组后的分析循环；返回值仍为 `{ analyses, failures, cancelled }`。

- [x] **Step 1: 写任务 2 分析状态的失败测试**

在 `app/src/app.test.ts` 增加一个 `makePreview(1)` 的增强直转换测试。等待分析开始后，断言任务 2 卡片存在 `[data-role="analysis-message"]`，任务 1 卡片不存在该节点；同时断言取消按钮仍可用。

```ts
it('keeps enhanced analysis progress in the originating Task 2 slot', async () => {
  const readDeferred = createDeferred<number[]>();
  const services = makeMockServices({
    loadDesktopState: vi.fn().mockResolvedValue(makeDesktopState({
      conversion_mode: 'direct',
      enhanced_mode: true,
    })),
    loadScanResult: vi.fn().mockResolvedValue([makePreview(1)]),
    readAudioFile: vi.fn().mockReturnValue(readDeferred.promise),
  });
  const root = document.createElement('div');
  bindApp(root, makeViewState({ conversionMode: 'direct', enhancedMode: true }), services);

  (root.querySelector('[data-action="start-all"]') as HTMLButtonElement).click();
  await vi.waitFor(() => expect(root.querySelector('[data-action="cancel-analysis"]')).not.toBeNull());

  expect(root.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
  expect(root.querySelector('[data-slot="1"] [data-role="analysis-message"]')).not.toBeNull();
  readDeferred.resolve([]);
});
```

- [x] **Step 2: 运行失败测试确认当前硬编码行为**

Run: `cd "/Users/mac2/Documents/W4DJ RKB/app" && /Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner app/src/app.test.ts -t "originating Task 2"`

Expected: FAIL，当前实现把分析节点渲染到 `[data-slot="0"]`。

- [x] **Step 3: 扩展分析状态并按槽分组处理候选歌曲**

给 `AppAnalysisState` 增加明确槽位字段，并让默认状态不属于任何槽：

```ts
export type AppAnalysisState = {
  slotIndex: SyncSlotIndex | null;
  status: 'idle' | 'running' | 'completed' | 'cancelled' | 'error';
  completed: number;
  total: number;
  resultCount: number;
  failedCount: number;
  message: string;
  currentItem?: string;
  stage?: string;
  resumeAvailable?: boolean;
};
```

在 `analyzePreviewCandidates` 开始处按 `preview.slot_index` 生成工作组，去重仍以 `source_path` 为键，但每个去重后的候选保留所属 `slotIndex`。分析循环进入某个工作组时重置该槽的 `completed/total`，把 `analysisState.slotIndex` 设为该组槽位；所有 `currentItem`、Worker progress、成功计数和失败计数更新继续走现有状态对象。结果和失败数组继续合并返回，不改变后端调用参数。

分组代码保持在前端编排层，不把槽位信息塞进 Worker 协议：

```ts
const groups = new Map<SyncSlotIndex, AppPreviewCandidate[]>();
for (const preview of previews) {
  const group = groups.get(preview.slot_index) || [];
  const seen = new Set(group.map((candidate) => candidate.source_path));
  for (const candidate of preview.preview.candidates) {
    if (!seen.has(candidate.source_path)) {
      group.push(candidate);
      seen.add(candidate.source_path);
    }
  }
  groups.set(preview.slot_index, group);
}

for (const [slotIndex, group] of groups) {
  analysisState = {
    ...analysisState,
    slotIndex,
    completed: 0,
    total: group.length,
    currentItem: '',
    stage: 'preparing',
  };
  // 复用现有 candidate 分析体，结果继续写入共同的 results/failures 数组。
}
```

- [x] **Step 4: 运行任务 2 回归测试确认通过**

Run: `cd "/Users/mac2/Documents/W4DJ RKB/app" && /Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner app/src/app.test.ts -t "originating Task 2"`

Expected: PASS。

### Task 2: 按槽渲染和增量更新分析进度

**Files:**
- Modify: `/Users/mac2/Documents/W4DJ RKB/app/src/app.ts:1171,1360-1371,1525-1622,1880-1907`
- Test: `/Users/mac2/Documents/W4DJ RKB/app/src/app.test.ts`

**Interfaces:**
- Consumes: `AppAnalysisState.slotIndex`、现有 `renderApp`/`renderSyncSlot` 和 `updateAnalysisProgressDom`。
- Produces: 任务 1/任务 2 均能独立显示分析文本、计数和进度条，未匹配槽位保持原有转换进度。

- [x] **Step 1: 写纯渲染回归测试**

把现有“增强分析显示在任务 1”的测试改为显式传入 `slotIndex: 0`，并新增 `slotIndex: 1` 场景：

```ts
it('renders analysis progress only in Task 2 when slotIndex is 1', () => {
  const root = renderApp(
    makeViewState({ enhancedMode: true }),
    null, null, null, [], null, false, null, false, false, false, 0,
    {
      slotIndex: 1,
      status: 'running',
      completed: 2,
      total: 9,
      resultCount: 0,
      failedCount: 0,
      message: '正在计算 BPM、Key 和响度',
      currentItem: 'Song.flac',
      stage: 'basic',
      resumeAvailable: false,
    },
  );

  expect(root.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
  expect(root.querySelector('[data-slot="1"] [data-role="analysis-message"]')?.textContent)
    .toContain('2/9');
  expect((root.querySelector('[data-slot="1"] [data-role="analysis-progress"]') as HTMLElement).style.width)
    .toBe('22%');
});
```

- [x] **Step 2: 运行纯渲染测试确认失败**

Run: `cd "/Users/mac2/Documents/W4DJ RKB/app" && /Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner app/src/app.test.ts -t "Task 2 when slotIndex is 1"`

Expected: FAIL，当前 `renderSyncSlot` 与 `renderApp` 把分析状态限定在任务 1。

- [x] **Step 3: 实现槽位路由和 DOM 定位**

在 `renderSyncSlot` 中把 `slotIndex === 0` 改为 `analysisState.slotIndex === slotIndex`；`renderApp` 将同一分析状态传给两个槽，由匹配条件决定只有一个槽显示分析内容。`updateAnalysisProgressDom` 改为先按 `analysisState.slotIndex` 找到任务卡，再在该卡内查找分析文本和进度条：

```ts
const analysisSlot = analysisState.slotIndex === null
  ? null
  : root.querySelector<HTMLElement>(
      `[data-role="sync-slot"][data-slot="${analysisState.slotIndex}"]`,
    );
const messageElement = analysisSlot?.querySelector<HTMLElement>('[data-role="analysis-message"]');
const progressElement = analysisSlot?.querySelector<HTMLElement>('[data-role="analysis-progress"]');
```

保持 `updateAnalysisProgressDom` 不调用 `render()`，并让分析终态在现有低频路径中清理 `slotIndex` 或恢复到对应槽的转换状态。

- [x] **Step 4: 运行前端完整测试**

Run: `cd "/Users/mac2/Documents/W4DJ RKB/app" && /Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner`

Expected: 所有现有测试和新增槽位测试 PASS。

### Task 3: 双任务边界验收与文档记录

**Files:**
- Modify: `/Users/mac2/Documents/W4DJ RKB/app/src/app.test.ts`
- Modify: `/Users/mac2/Documents/W4DJ RKB/计划.md`
- Modify: `/Users/mac2/Documents/W4DJ RKB/docs/project-state.md`
- Modify: `/Users/mac2/Documents/W4DJ RKB/docs/handoff.md`

**Interfaces:**
- Consumes: Task 1/2 的槽位感知分析状态和渲染。
- Produces: 双任务同时启动时分析进度不污染另一槽，文档记录修复和验收边界。

- [x] **Step 1: 增加双任务分析隔离测试**

构造 `makePreview(0)` 和 `makePreview(1)` 的同批次输入，确认分析分组依次切换槽位；任一时刻当前分析节点只出现在对应 `slotIndex`，两个任务的转换完成状态和取消按钮不互相覆盖。

```ts
it('routes a two-slot analysis batch to one originating slot at a time', () => {
  const taskOne = renderApp(
    makeViewState({ enhancedMode: true }),
    null, null, null, [], null, false, null, false, false, false, 0,
    {
      slotIndex: 0,
      status: 'running',
      completed: 1,
      total: 1,
      resultCount: 0,
      failedCount: 0,
      message: '正在分析任务 1',
      currentItem: 'one.flac',
      stage: 'basic',
      resumeAvailable: false,
    },
  );
  expect(taskOne.querySelector('[data-slot="0"] [data-role="analysis-message"]')).not.toBeNull();
  expect(taskOne.querySelector('[data-slot="1"] [data-role="analysis-message"]')).toBeNull();

  const taskTwo = renderApp(
    makeViewState({ enhancedMode: true }),
    null, null, null, [], null, false, null, false, false, false, 0,
    {
      slotIndex: 1,
      status: 'running',
      completed: 1,
      total: 1,
      resultCount: 0,
      failedCount: 0,
      message: '正在分析任务 2',
      currentItem: 'two.flac',
      stage: 'basic',
      resumeAvailable: false,
    },
  );
  expect(taskTwo.querySelector('[data-slot="0"] [data-role="analysis-message"]')).toBeNull();
  expect(taskTwo.querySelector('[data-slot="1"] [data-role="analysis-message"]')).not.toBeNull();
});
```

- [x] **Step 2: 运行完整验证**

Run:

```bash
cd "/Users/mac2/Documents/W4DJ RKB"
git diff --check
cargo fmt --all -- --check
cd app
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vitest/vitest.mjs run --config vitest.config.ts --configLoader runner
/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/vite/bin/vite.js build --config vite.config.ts --configLoader runner --outDir /private/tmp/w4dj-vite-build-analysis-slot
```

Expected: diff、格式、前端测试和 Vite 构建全部通过；只保留已有 bundle 体积提示。

- [x] **Step 3: 更新项目状态**

在 `计划.md`、`docs/project-state.md` 和 `docs/handoff.md` 中记录：增强分析进度按 `slot_index` 显示，任务 2 不再落到任务 1；真实长音频人工验收仍需在桌面 App 中确认。不要修改版本号，不提交或推送。
