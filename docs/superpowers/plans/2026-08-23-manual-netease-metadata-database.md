# 手动选择网易云元数据数据库 Implementation Plan

> **2026-08-24 验收入口更新：** 真实数据库选择、只读快照、FLAC/MP3 写回和 ExifTool 验收改用 `2026-08-24-headless-acceptance.md` 的后台参数与场景；不再依赖任务 1 GUI 控件完成验收。

## Execution status (2026-08-23)

Tasks 1–5 are implemented in the shared worktree. Manual selection now uses
strict read-only schema validation, persists only after validation, restores the
previous preference when persistence fails, and exposes camelCase status DTOs
through dedicated Tauri commands. Task 1 renders the selection/automatic-
location controls beside the source picker without invoking scan, conversion,
analysis, or Dashboard refresh; conversion and analysis writeback continue to
share one immutable resolver per batch. Task 6 automation is complete. Real
database/FLAC ExifTool acceptance remains environment-dependent and must be
performed with the user's actual `sqlite_storage.sqlite3` and output files.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在任务 1 来源标题右侧提供持久化的网易云数据库选择入口，并让普通转换、仅更新元数据和增强分析写回统一使用该只读数据库完成歌曲匹配与本地封面恢复。

**Architecture:** 在现有 `netease_database_path` 偏好和 `NeteaseMetadataResolver` 之上增加与 Dashboard 解耦的状态 DTO 与三条 Tauri 命令。前端用独立 UI 状态呈现选择、更换和恢复自动定位；每个任务批次只加载一次不可变 resolver，所有元数据路径共享该快照。

**Tech Stack:** Rust、Tauri 2、Serde、Rusqlite、TypeScript、Vite、Vitest、原生 Tauri 文件选择器。

## Global Constraints

- 网易云数据库必须使用 SQLite read-only 打开；不得建表、迁移、写入、VACUUM、删除或重命名。
- 手动路径优先；无效选择不得覆盖旧值。已保存路径后来失效时回退自动定位并显示警告，但不自动删除偏好。
- 选择、清除或加载状态不得启动扫描、转换、增强分析或歌曲库刷新。
- Dashboard 继续只查询 `w4dj.sqlite3`；不得把网易云数据库或 `library-dashboard.sqlite3` 恢复为 Dashboard 数据源。
- 不联网下载远程 `picUrl`，不覆盖已有可靠标签或已有有效内嵌封面。
- DTO 和 Tauri 载荷使用 camelCase；新增字段保持旧偏好和旧调用兼容。
- 不修改版本号，不新增 baseline、hash、冻结 contract 或发布 gate。
- 不 commit、push、merge 或 release；仓库规则要求等待用户最终确认。本计划中的每个 Task 完成后只记录验证结果并进入下一项。

## File Structure

- `src/netease.rs`：严格只读加载单个手动数据库，构建不可变批次解析器。
- `src-tauri/src/main.rs`：专用状态 DTO、选择/清除命令、偏好原子保存和分析写回上下文接线。
- `src/preferences.rs`、`tests/preferences_roundtrip.rs`：复用并验证 `netease_database_path` 的向后兼容持久化。
- `src/sync.rs`：保持所有元数据更新函数显式接收 `ConversionMetadataContext`。
- `app/src/app.ts`：服务接口、任务 1 数据库 UI 状态、按钮、交互和翻译。
- `app/src/app.test.ts`：任务 1 渲染、选择/取消/清除、无副作用和窄窗口行为测试。
- `app/src/styles.css`：来源标题操作组和数据库状态样式。
- `计划.md`、`docs/project-state.md`、`docs/handoff.md`：记录功能边界、实现状态与验收结果。

---

### Task 1: 严格只读的手动数据库加载入口

**Files:**
- Modify: `src/netease.rs:181-235` (`impl NeteaseMetadataResolver`)
- Test: `src/netease.rs:1900-2050` (`mod tests`)

**Interfaces:**
- Consumes: 现有 `has_supported_netease_table(path)`、`load_records_from_db(path)`。
- Produces:

```rust
impl NeteaseMetadataResolver {
    pub fn load_exact(database_path: &Path) -> io::Result<Self>;
}
```

- [ ] **Step 1: 写严格加载失败测试**

在 `src/netease.rs` 测试模块创建一个只有无关表的 SQLite 文件，断言 `load_exact` 返回错误，且不会自动定位另一份数据库：

```rust
#[test]
fn exact_resolver_rejects_unsupported_schema_without_fallback() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("wrong.sqlite3");
    Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (id INTEGER);")
        .unwrap();

    let error = NeteaseMetadataResolver::load_exact(&database).unwrap_err();
    assert!(error.to_string().contains("schema"));
}
```

- [ ] **Step 2: 运行测试并确认红灯**

Run: `cargo test --lib netease::tests::exact_resolver_rejects_unsupported_schema_without_fallback -- --exact`

Expected: FAIL，因为 `load_exact` 尚不存在。

- [ ] **Step 3: 实现严格加载**

在 `NeteaseMetadataResolver` 中增加：

```rust
pub fn load_exact(database_path: &Path) -> io::Result<Self> {
    if !database_path.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "网易云数据库文件不存在"));
    }
    let supported = has_supported_netease_table(database_path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !supported {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "网易云数据库 schema 不受支持",
        ));
    }
    let records = load_records_from_db(database_path)?;
    Ok(Self {
        database_path: Some(database_path.to_path_buf()),
        records: Arc::new(records),
        database_loaded: true,
        warning: None,
    })
}
```

让 `load_with_warning` 对有效候选调用 `load_exact`，但保留“手动路径失效后自动回退”的现有策略。

- [ ] **Step 4: 增加只读与快照测试**

复用现有支持表 fixture，记录数据库大小和修改时间；加载后确认二者不变。加载完成后删除 fixture 数据库，再调用 `resolver.recover(&source)`，确认内存快照仍可匹配。

- [ ] **Step 5: 运行 netease 单元测试**

Run: `cargo test --lib netease --no-fail-fast`

Expected: 所有 `netease` 测试通过；数据库文件没有 `-wal`/`-shm` 写入副作用。

---

### Task 2: 与 Dashboard 解耦的数据库状态和持久化命令

**Files:**
- Modify: `src-tauri/src/main.rs:850-920`（DTO）
- Modify: `src-tauri/src/main.rs:3444-3470`（resolver/status helpers）
- Modify: `src-tauri/src/main.rs:4087-4125`（命令）
- Modify: `src-tauri/src/main.rs:5200-5250`（invoke handler）
- Modify: `src-tauri/src/main.rs:5440-5465`（偏好保存）
- Test: `src-tauri/src/main.rs:6800-6960`（Tauri 单元测试）
- Test: `tests/preferences_roundtrip.rs`

**Interfaces:**
- Consumes: `NeteaseMetadataResolver::load_exact`、`load_with_warning`、`AppPreferences.netease_database_path`。
- Produces:

```rust
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum NeteaseMetadataDatabaseSource {
    Manual,
    Automatic,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct NeteaseMetadataDatabaseStatus {
    manual_path: Option<String>,
    effective_path: Option<String>,
    source: NeteaseMetadataDatabaseSource,
    loaded: bool,
    record_count: usize,
    warning: Option<String>,
}
```

```rust
#[tauri::command]
fn load_netease_metadata_database_status(
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataDatabaseStatus, String>;

#[tauri::command]
fn select_netease_metadata_database(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataDatabaseStatus, String>;

#[tauri::command]
fn clear_netease_metadata_database(
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataDatabaseStatus, String>;
```

- [ ] **Step 1: 写 DTO 序列化和状态来源测试**

```rust
#[test]
fn netease_metadata_database_status_uses_camel_case() {
    let value = serde_json::to_value(NeteaseMetadataDatabaseStatus {
        manual_path: Some("/music/db.sqlite3".into()),
        effective_path: Some("/music/db.sqlite3".into()),
        source: NeteaseMetadataDatabaseSource::Manual,
        loaded: true,
        record_count: 42,
        warning: None,
    }).unwrap();
    assert_eq!(value["source"], "manual");
    assert_eq!(value["recordCount"], 42);
    assert!(value.get("manual_path").is_none());
}
```

另写 helper 测试覆盖 `manual`、失效手动路径回退后的 `automatic + warning`、无数据库时的 `unavailable`。

- [ ] **Step 2: 运行 Tauri 定点测试并确认红灯**

Run: `cargo test --manifest-path src-tauri/Cargo.toml netease_metadata_database_status -- --nocapture`

Expected: FAIL，因为 DTO/helper 尚不存在。

- [ ] **Step 3: 提取无 Tauri 依赖的状态 helper**

新增：

```rust
fn resolve_netease_metadata_database_status(
    manual_path: Option<&Path>,
) -> Result<(NeteaseMetadataDatabaseStatus, NeteaseMetadataResolver), String> {
    let (resolver, warning) = NeteaseMetadataResolver::load_with_warning(manual_path)
        .map_err(|error| format!("网易云数据库加载失败：{error}"))?;
    let effective = resolver.database_path().map(Path::to_path_buf);
    let source = if manual_path.is_some() && effective.as_deref() == manual_path {
        NeteaseMetadataDatabaseSource::Manual
    } else if effective.is_some() {
        NeteaseMetadataDatabaseSource::Automatic
    } else {
        NeteaseMetadataDatabaseSource::Unavailable
    };
    let status = NeteaseMetadataDatabaseStatus {
        manual_path: manual_path.map(|path| path.display().to_string()),
        effective_path: effective.map(|path| path.display().to_string()),
        source,
        loaded: resolver.database_loaded(),
        record_count: resolver.record_count(),
        warning,
    };
    Ok((status, resolver))
}
```

- [ ] **Step 4: 让偏好保存可以报告失败并回滚内存状态**

把现有保存主体提取为：

```rust
fn persist_preferences_checked(state: &AppState) -> Result<(), String> {
    // 合并 Desktop preferences 与 manual_database_path 后调用 save_preferences。
    // preferences_path 为空时返回 Ok(())，其他 I/O 错误转换为中文错误。
}
```

现有 `persist_preferences` 保留为记录错误的兼容包装。选择/清除命令在持有旧路径副本的情况下更新锁，保存失败时恢复旧值并返回错误。

- [ ] **Step 5: 实现三个专用命令**

选择命令必须先执行：

```rust
let selected = PathBuf::from(path.trim());
let resolver = NeteaseMetadataResolver::load_exact(&selected)
    .map_err(|error| format!("所选网易云数据库无效：{error}"))?;
```

只有校验成功后才更新 `manual_database_path`。保存后返回 `source=manual`、实际路径和记录数。清除命令只清除偏好并重新解析自动状态，不调用任何刷新函数。

现有 `select_netease_database_fallback` 与 `clear_netease_database_fallback` 保留兼容，但委托同一验证/保存 helper，避免两套规则分叉。

- [ ] **Step 6: 注册命令并验证偏好 roundtrip**

在 `tauri::generate_handler!` 加入三条命令。扩展 `tests/preferences_roundtrip.rs`，确认旧 JSON 没有字段时读取为 `None`，普通设置保存不会清掉已选路径。

Run:

```bash
cargo test --test preferences_roundtrip
cargo test --manifest-path src-tauri/Cargo.toml netease_metadata_database -- --nocapture
```

Expected: 全部通过；错误 schema 和保存失败测试中旧路径保持不变。

---

### Task 3: 所有元数据写回路径共享批次解析器

**Files:**
- Modify: `src-tauri/src/main.rs:2949-3130` (`apply_track_analysis_results`)
- Modify: `src-tauri/src/main.rs:3453-3469` (`conversion_metadata_context`)
- Modify: `src-tauri/src/main.rs:5601-5960` (`run_confirmed_sync_task`)
- Modify: `src/sync.rs:1260-1710`（context-aware 元数据函数）
- Test: `src/sync.rs:4450-4570`
- Test: `src-tauri/src/main.rs` test module

**Interfaces:**
- Consumes:

```rust
pub struct ConversionMetadataContext {
    pub netease: Arc<NeteaseMetadataResolver>,
}
```

- Produces: 普通转换、metadata-only、批次内分析写入和后置 `apply_track_analysis_results` 全部显式使用同一 `Arc<ConversionMetadataContext>`。

- [ ] **Step 1: 写所选数据库贯穿分析回写的失败测试**

创建支持表数据库，记录一首源文件的标题/歌手/专辑和本地 JPEG。构建 resolver/context，对临时 FLAC 调用 context-aware 分析写回，再读取输出标签，断言数据库字段与图片存在。

测试还要构建两个 resolver（A/B 各有不同专辑名），确认一个已创建的 context 在偏好模拟切换到 B 后仍写入 A，证明批次快照不可变。

- [ ] **Step 2: 运行 sync 定点测试并确认缺口**

Run: `cargo test --lib sync::tests::selected_netease_database_is_used_by_analysis_writeback -- --exact`

Expected: 初始 FAIL，暴露 `apply_track_analysis_results` 仍调用无 context 的 `apply_track_analysis_metadata`。

- [ ] **Step 3: 修复后置分析写回**

在 `apply_track_analysis_results` 开头只创建一次：

```rust
let metadata_context = conversion_metadata_context(&state);
```

把事务闭包改为：

```rust
let result = update_analysis_metadata_transactionally(destination_path, |temporary_output| {
    apply_track_analysis_metadata_with_context(
        temporary_output,
        &embedded_analysis,
        metadata_context.as_ref(),
    )
});
```

- [ ] **Step 4: 审计四条调用链并删除隐式默认解析器**

使用：

```bash
rg -n "apply_track_analysis_metadata\(|update_existing_metadata_transactionally\(|ensure_output_metadata_with_settings\(" src-tauri/src/main.rs src/sync.rs
```

逐项确认生产调用属于以下两类之一：显式传递 `ConversionMetadataContext`；或仅为兼容测试包装。不得在逐曲循环中调用 `NeteaseMetadataResolver::load`。

- [ ] **Step 5: 验证失败隔离和封面优先级**

增加测试：数据库无匹配时分析基础字段仍可写入；数据库只有远程 `picUrl` 时不联网；输出已有有效内嵌封面时不被数据库低优先级封面覆盖；歧义匹配不写猜测结果。

Run: `cargo test --lib sync --no-fail-fast`

Expected: 全部通过。

---

### Task 4: 前端服务、状态加载和任务 1 操作组

**Files:**
- Modify: `app/src/app.ts:350-445` (`AppServices`)
- Modify: `app/src/app.ts:500-760`（翻译）
- Modify: `app/src/app.ts:1150-1180` (`tauriServices`)
- Modify: `app/src/app.ts:1212-1420` (`renderApp`)
- Modify: `app/src/app.ts:1580-1730` (`renderSyncSlot`)
- Modify: `app/src/app.ts:1880-2000` (`bindApp` local state)
- Modify: `app/src/app.ts:3270-3390`（交互 helpers）
- Modify: `app/src/app.ts:4580-4660`（click dispatcher）
- Modify: `app/src/styles.css:860-930`
- Test: `app/src/app.test.ts:360-400`
- Test: `app/src/app.test.ts:1090-1220`

**Interfaces:**
- Consumes: Task 2 三条 Tauri 命令。
- Produces:

```ts
export type NeteaseMetadataDatabaseStatus = {
  manualPath: string | null;
  effectivePath: string | null;
  source: 'manual' | 'automatic' | 'unavailable';
  loaded: boolean;
  recordCount: number;
  warning: string | null;
};

export type NeteaseMetadataDatabaseUiState = {
  status: NeteaseMetadataDatabaseStatus | null;
  busy: boolean;
  message: string | null;
  error: string | null;
};
```

`AppServices` 增加：

```ts
loadNeteaseMetadataDatabaseStatus?: () => Promise<NeteaseMetadataDatabaseStatus>;
selectNeteaseMetadataDatabase?: (path: string) => Promise<NeteaseMetadataDatabaseStatus>;
clearNeteaseMetadataDatabase?: () => Promise<NeteaseMetadataDatabaseStatus>;
pickNeteaseDatabase?: () => Promise<string | null>;
```

- [ ] **Step 1: 写任务 1 独占渲染测试**

调用 `renderApp` 并断言：

```ts
expect(root.querySelectorAll('[data-action="select-netease-database"]')).toHaveLength(1);
expect(root.querySelector('[data-slot="1"] [data-action="select-netease-database"]')).toBeNull();
expect(root.querySelector('[data-action="scan-local-netease"]')).not.toBeNull();
```

传入 `manualPath: '/music/sqlite_storage.sqlite3'` 后，断言界面只显示 `sqlite_storage.sqlite3`，并出现 `data-action="clear-netease-database"`，不显示完整路径。

- [ ] **Step 2: 运行前端定点测试并确认红灯**

Run: `pnpm --dir app test -- --run app/src/app.test.ts`

Expected: 新测试 FAIL，因为专用状态和按钮尚不存在。

- [ ] **Step 3: 接入服务和初始状态加载**

在 `tauriServices` 中使用：

```ts
loadNeteaseMetadataDatabaseStatus: () =>
  invoke<NeteaseMetadataDatabaseStatus>('load_netease_metadata_database_status'),
selectNeteaseMetadataDatabase: (path) =>
  invoke<NeteaseMetadataDatabaseStatus>('select_netease_metadata_database', { path }),
clearNeteaseMetadataDatabase: () =>
  invoke<NeteaseMetadataDatabaseStatus>('clear_netease_metadata_database'),
```

`bindApp` 在桌面状态 hydration 完成后加载一次状态。失败只更新数据库 UI 错误，不阻止应用启动或清空任务状态。

- [ ] **Step 4: 渲染紧凑操作组**

把当前单个扫描按钮包装为：

```html
<div class="netease-source-actions" data-role="netease-source-actions">
  <button data-action="scan-local-netease">…</button>
  <button data-action="select-netease-database">…</button>
  <!-- manualPath 非空时才渲染 clear 按钮 -->
</div>
```

数据库按钮选择后显示 `basename(manualPath)`；`title` 可显示完整路径供用户悬停查看。增加 `neteaseDatabaseBusy`，只禁用选择/清除按钮。

- [ ] **Step 5: 实现选择、取消和清除处理**

```ts
const selectNeteaseMetadataDatabase = async () => {
  if (neteaseMetadataDatabase.busy || !services.pickNeteaseDatabase
      || !services.selectNeteaseMetadataDatabase) return;
  const path = await services.pickNeteaseDatabase();
  if (!path) return;
  // 设置 busy，调用后端，成功保存 status/message，失败保存 error，finally 清 busy。
};
```

清除 handler 只调用 `clearNeteaseMetadataDatabase`。两者都不得调用 `locateNeteaseLibrary`、`selectSourceDirectory`、`startScan`、`previewAllSync`、`startConfirmedSync` 或任何 Dashboard service。

- [ ] **Step 6: 加入中英文文案和响应式样式**

中文：`选择网易云数据库`、`恢复自动定位`、`已选择数据库，开始转换或分析时使用`。英文：`Choose NetEase database`、`Use automatic location`、`Database selected; it will be used when conversion or analysis starts`。

`.netease-source-actions` 使用 `display:flex; flex-wrap:wrap; justify-content:flex-end; gap:6px;`。复用 `.netease-scan-button` 的字体和焦点样式，不引入新颜色体系。

- [ ] **Step 7: 运行前端定点测试**

Run: `pnpm --dir app test -- --run app/src/app.test.ts`

Expected: 任务 1 数据库相关测试和既有网易云目录扫描测试全部通过。

---

### Task 5: 前端无副作用、错误恢复和竞态测试

**Files:**
- Modify: `app/src/app.test.ts`
- Modify if required by tests: `app/src/app.ts`

**Interfaces:**
- Consumes: Task 4 `NeteaseMetadataDatabaseUiState` 与三个 service。
- Produces: 可证明选择行为不触发其他任务的 UI 契约。

- [ ] **Step 1: 测试文件选择器取消**

`pickNeteaseDatabase` 返回 `null`；断言 `selectNeteaseMetadataDatabase` 未调用，状态文案不变化。

- [ ] **Step 2: 测试成功选择无副作用**

选择器返回 `/music/sqlite_storage.sqlite3`，后端返回 `source:'manual'`。断言选择只调用一次，并逐一断言以下 mock 未调用：

```ts
expect(services.locateNeteaseLibrary).not.toHaveBeenCalled();
expect(services.selectSourceDirectory).not.toHaveBeenCalled();
expect(services.startScan).not.toHaveBeenCalled();
expect(services.previewAllSync).not.toHaveBeenCalled();
expect(services.startConfirmedSync).not.toHaveBeenCalled();
expect(services.refreshLibraryCatalog).not.toHaveBeenCalled();
```

- [ ] **Step 3: 测试无效选择保留旧状态**

初始手动路径为 `old.sqlite3`，选择命令 reject `schema 不受支持`。断言按钮仍显示 `old.sqlite3`，错误文案可见，并可以再次点击重试。

- [ ] **Step 4: 测试恢复自动定位和重复点击**

清除成功返回 `source:'automatic'`；断言清除按钮消失、自动实际路径不作为“手动选择”显示。选择 Promise 挂起时连续点击两次，只触发一次文件选择和一次后端调用。

- [ ] **Step 5: 测试窄窗口与语言切换**

断言按钮组存在稳定 `data-role`，CSS 中包含 `flex-wrap: wrap`；切换英文后按钮文字更新，当前文件名保持不变。

- [ ] **Step 6: 运行完整前端测试和构建**

Run:

```bash
pnpm --dir app test -- --run
pnpm --dir app build
```

Expected: 所有 Vitest 测试通过；Vite 生产构建成功且无 TypeScript 错误。

---

### Task 6: 诊断、文档和完整验收

**Files:**
- Modify: `src/history.rs`
- Modify: `tests/history.rs`
- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`
- Modify: `docs/superpowers/plans/2026-08-23-manual-netease-metadata-database.md`（勾选执行结果）

**Interfaces:**
- Consumes: `NeteaseRecoveryDiagnostic`、手动导出错误报告、Tasks 1–5 的状态与批次解析器。
- Produces: 手动报告能够解释实际数据库、匹配方式和封面来源；项目文档准确反映功能边界。

- [ ] **Step 1: 写报告字段测试**

在 `tests/history.rs` 构建含 `NeteaseRecoveryDiagnostic` 的历史条目，断言手动报告包含：数据库实际路径、`manual/automatic` 来源、记录数、`exactPath/ambiguous/noMatch`、歌曲 ID、专辑 ID、`embedded/databaseBlob/explicitLocalPath/localCache/remoteOnly/missing/invalid` 和终止原因。

- [ ] **Step 2: 运行 history 测试并补齐缺失字段**

Run: `cargo test --test history`

Expected: 初始测试精确指出尚未输出的字段；只扩展手动报告，不自动生成或覆盖用户报告。

- [ ] **Step 3: 更新项目文档**

记录：任务 1 新入口、选择只保存、批次快照、手动路径优先/失效回退、Dashboard 数据边界、远程封面不下载，以及真实验收是否完成。不得把“数据库校验通过”写成“真实封面恢复已验收”。

- [ ] **Step 4: 运行 Rust 全量验证**

Run:

```bash
cargo test --all
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Expected: 测试、格式、check 和 diff-check 通过。若严格 Clippy 仅被共享工作树既有 `dead_code` 阻塞，记录精确文件、行号和 lint，不隐藏失败。

- [ ] **Step 5: 执行真实数据库验收**

在最新 App 中点击任务 1“选择网易云数据库”，选择用户真实 `sqlite_storage.sqlite3`。确认只显示文件名且任务未启动。随后对一首有 `meta` 本地封面的 FLAC 和一首 MP3 分别执行普通转换/仅更新元数据或增强分析写回。

Run:

```bash
exiftool -Title -Artist -Album -Picture -BPM -InitialKey -Genre '/absolute/output.flac'
exiftool -Title -Artist -Album -Picture -BPM -InitialKey -Genre '/absolute/output.mp3'
```

Expected: 匹配字段正确，封面存在；已有封面不被降级；手动导出的错误报告显示实际数据库、匹配方式和封面来源。恢复自动定位后再次加载界面，不再显示手动路径。

- [ ] **Step 6: 编译最新 macOS App**

先使用仓库当前约定的 Tauri build 命令；若当前目标为 Apple Silicon：

```bash
pnpm --dir app build
cargo tauri build --target aarch64-apple-darwin --bundles app --no-sign
```

Expected: 生成可启动的 `W4DJ RKB.app`。最终报告提供产物的绝对可点击路径。

- [ ] **Step 7: 最终状态报告**

Run:

```bash
git status --short --branch
git diff --stat
```

报告已完成任务、测试数量、真实验收、环境限制、App 路径、`git status` 和 `git diff --stat`。不提交或推送，等待用户确认。

## 2026-08-24 验收入口迁移

本计划后续真实数据库/音频回读改由隐藏后台场景执行，数据库保持只读；不再使用 GUI/可访问性点击触发验收。
