# 重复歌曲专辑消歧 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. 每个任务都必须先写失败测试，再实现，再运行对应验证。不得提交或推送。

**Goal:** 让不同专辑的同名歌曲同时保留；仅在真实输出名冲突的歌曲组内增加最短必要的消歧后缀，所有不重复歌曲的文件名保持完全不变，并完整保留音频元数据。

**Architecture:** 预检阶段先读取每个候选的歌曲身份（网易云 `trackId/albumId`、专辑、标题、歌手），按现有规则生成基础文件名。只有多个不同身份映射到同一目标路径时，才在该冲突组内追加专辑名或稳定 ID；转换写入的 Title、Artist、Album、封面和其他已有元数据仍使用原始元数据，不把文件名后缀写入标签。W4DJ 数据库以最终 `destination_path` 记录每首输出，并保存可用的网易云身份字段。

**Tech Stack:** Rust/Tauri、SQLite、`NeteaseRecord`、现有 preview/sync/metadata 流程、Rust integration tests、ExifTool 人工回读。

## Global Constraints

- 规则只适用于同一批次内实际生成相同目标文件名的歌曲组；普通歌曲不得新增专辑后缀。
- 不覆盖、不丢弃不同网易云 `trackId`/`albumId` 的歌曲；两首都必须生成独立输出文件。
- 文件名消歧不改变音频内部 Title、Artist、Album、封面、歌词和分析元数据。
- 优先使用 `trackId + albumId` 区分歌曲；没有 ID 时使用专辑、标题、歌手和稳定源文件指纹顺序兜底。
- 后缀只在冲突组内出现，优先使用 `[专辑名]`；专辑仍相同或缺失时使用稳定短 ID，最后才使用确定性的 `(2)`、`(3)`。
- 旧版数据库和旧版 `w4dj.sqlite3` 记录必须可读取；新增身份字段使用可空列和向后兼容迁移。
- 不修改版本号，不修改网易云源数据库，不删除或重命名已有用户文件，不 commit、push、merge 或 release。

---

### Task 1: 建立可比较的歌曲身份

**Files:**
- Modify: `src/netease.rs`
- Modify: `src/sync.rs`
- Modify: `src/w4dj_library.rs`
- Modify: `src/library_catalog.rs`（仅在现有写回 DTO 缺少字段时同步可选字段）
- Test: `tests/netease.rs`, `tests/w4dj_library.rs`, `tests/sync_policy.rs`

**Interfaces:**
- 新增内部身份类型：

```rust
pub struct OutputTrackIdentity {
    pub track_id: Option<String>,
    pub album_id: Option<String>,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub source_path: std::path::PathBuf,
}
```

- 新增 `OutputTrackIdentity::stable_key()`：有 `track_id` 时返回 `track_id`；否则返回规范化的 `title + artists + album + source_path`。
- `NeteaseRecord.track_id`、`NeteaseRecord.album_id`、标题、歌手和专辑必须进入转换候选；MP3/FLAC 标签缺失时才使用源文件名兜底。
- `w4dj.sqlite3` 的 `tracks` 增加可空 `netease_album_id TEXT`，保留现有 `netease_track_id`；旧库启动时通过 `ALTER TABLE ... ADD COLUMN` 兼容迁移。

- [ ] **Step 1: 写失败测试**
  - 构造两个标题、歌手相同但 `track_id/album_id/album` 不同的 `OutputTrackIdentity`，断言 `stable_key()` 不相同。
  - 构造同一 `track_id` 的两个来源，断言 `stable_key()` 相同。
  - 用旧版无 `netease_album_id` 的临时 SQLite 打开并断言迁移后可读写。

- [ ] **Step 2: 运行失败测试**

```bash
cargo test --test netease --test w4dj_library --test sync_policy
```

预期：新身份类型、数据库字段或迁移尚不存在时失败。

- [ ] **Step 3: 实现身份传递和兼容迁移**
  - 从网易云读取结果复制 `track_id/album_id`，不得把 ID 丢在预检层之外。
  - 在 W4DJ upsert/query DTO 中加入可选 `netease_album_id`，旧 JSON/旧数据库读取为空即可。
  - 迁移只新增列和索引，不重写既有歌曲记录。

- [ ] **Step 4: 运行测试**

```bash
cargo test --test netease --test w4dj_library --test sync_policy
```

预期：全部通过，旧数据库 round-trip 不丢失其他元数据。

---

### Task 2: 只对冲突组生成最短消歧文件名

**Files:**
- Modify: `src/filename_policy.rs`
- Modify: `src/preview.rs`
- Modify: `src/sync.rs`
- Test: `tests/preview.rs`, `tests/sync_policy.rs`

**Interfaces:**

```rust
pub fn disambiguate_duplicate_output_names(
    candidates: &mut [PreviewCandidate],
    identities: &std::collections::HashMap<std::path::PathBuf, OutputTrackIdentity>,
);
```

- 输入是已经按现有规则生成的基础目标路径；输出只改变同路径冲突组的 `name/destination_path`。
- 单元素组必须原样返回，不能因为它有专辑字段就加后缀。
- 冲突组的后缀顺序固定为：
  1. 不同 `album_id` 或不同专辑名：`基础名 [清理后的专辑名]`；
  2. 专辑仍相同：`基础名 [track_id 的稳定短形式]`；
  3. 身份信息不足：按规范化源路径排序后追加 `(2+)`。
- 后缀本身通过现有文件名安全清理函数处理；若专辑名为空，不生成空括号。
- 生成后再次检查目标路径；仍冲突时使用稳定短 ID，禁止静默覆盖。

- [ ] **Step 1: 写失败测试**
  - 10 首普通歌曲无冲突：输出文件名与旧快照逐字一致，不带专辑后缀。
  - 两首同标题/同歌手、不同专辑：分别生成 `[专辑 A]`、`[专辑 B]`，两个目标路径不同。
  - 两首同标题/同歌手/同专辑、不同 `track_id`：使用稳定 ID，两个目标路径不同。
  - 完全相同身份重复来源：按确定性顺序生成基础名和 `(2)`，两者都不覆盖。
  - 专辑名含 `/`、NUL、控制字符：后缀安全清理，最终路径不含非法字符。

- [ ] **Step 2: 运行失败测试**

```bash
cargo test --test preview --test sync_policy
```

- [ ] **Step 3: 实现冲突组算法并接入预检**
  - 保留现有普通命名函数的结果作为第一阶段结果。
  - 将 `planned_paths` 的“直接报错”分支改为先收集冲突组，再调用消歧函数。
  - 只为冲突组修改 `candidate_name`；非冲突候选不得经过专辑后缀路径。
  - `Skip/Overwrite/UpdateMetadata` 继续适用于同一身份对应的已有目标文件；不同身份的同名候选必须走消歧，不能覆盖。
  - 预检结果中展示实际生成的两个目标路径，错误报告不再把可安全消歧的歌曲列为失败。

- [ ] **Step 4: 运行测试**

```bash
cargo test --test preview --test sync_policy
cargo fmt --all -- --check
```

---

### Task 3: 保证完整元数据与数据库记录独立保存

**Files:**
- Modify: `src/sync.rs`
- Modify: `src/metadata.rs`
- Modify: `src/w4dj_library.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `tests/w4dj_library.rs`, `tests/history.rs`, existing metadata/sync tests

**Interfaces:**
- 转换事务接收 `(source_path, destination_path, OutputTrackIdentity)`；文件名消歧只影响 `destination_path`。
- 元数据写回必须继续写入：Title、Artist、Album、Album Artist（如有）、Genre、年份、封面、歌词和现有 W4DJ 分析字段；不得从消歧文件名重新解析这些字段。
- W4DJ `tracks` 的唯一关联使用最终 `destination_path`/`local_files.path`，同时保存 `netease_track_id`、`netease_album_id` 和 `album`。

- [ ] **Step 1: 写失败测试**
  - 将两个不同专辑的同名候选转换到两个消歧路径，读取 SQLite，断言有两条 `local_files`、两个 album 值和两个身份 ID。
  - 用 ExifTool/测试元数据读取器断言两个输出的 Title/Artist/Album 与源记录一致，文件名后缀不出现在标签中。
  - 重复执行同一批次，断言不会覆盖另一专辑，也不会重复新增同身份记录。
  - 模拟数据库 upsert 失败，断言音频文件保留且任务报告明确记录数据库警告。

- [ ] **Step 2: 运行失败测试**

```bash
cargo test --test w4dj_library --test history
cargo test --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 3: 实现独立元数据写回和 upsert**
  - 将原始身份和最终路径分开传递，禁止从 `[专辑]` 或 `[track-id]` 后缀反推标签。
  - 写入 W4DJ SQLite 时保留两个输出记录；JSON 镜像若继续生成，也必须按最终路径更新。
  - 错误报告区分“转换失败”和“数据库写回警告”，不能把成功音频标记为转换失败。

- [ ] **Step 4: 运行测试**

```bash
cargo test --test w4dj_library --test history
cargo test --manifest-path src-tauri/Cargo.toml
```

---

### Task 4: 预览、历史和诊断展示

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `app/src/app.ts`
- Modify: `app/src/library-dashboard.ts`
- Modify: `app/src/styles.css`（仅增加冲突组提示，不改变普通歌曲布局）
- Test: `app/src/app.test.ts`, `app/src/library-dashboard.test.ts`, Tauri tests

**Steps:**

- [ ] **Step 1:** 为预览模型增加可选 `collisionGroup`、`disambiguationReason` 和最终目标路径字段；旧客户端缺字段时按普通候选渲染。
- [ ] **Step 2:** 在任务预览/运行会话中仅对冲突歌曲显示“同名歌曲，已按专辑区分”；普通歌曲不显示专辑后缀提示。
- [ ] **Step 3:** 历史和错误报告逐曲显示最终文件名、专辑、`trackId/albumId`（若有），让用户能确认两首都被保留。
- [ ] **Step 4:** 测试普通候选不出现消歧文案、冲突候选显示两条不同路径、旧历史 JSON 仍可渲染。

验证：

```bash
pnpm --dir app test -- --run app/src/app.test.ts app/src/library-dashboard.test.ts
pnpm --dir app build
```

---

### Task 5: 完整自动化与真实数据验收

**Files:**
- Modify: `计划.md`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`

**自动化验收：**

```bash
cargo test --test netease --test preview --test sync_policy --test w4dj_library --test history
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --all
pnpm --dir app test -- --run
pnpm --dir app build
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
git diff --check
```

**真实数据验收：**

- 使用包含 `STONE KOLD.ncm` 与 `STONE KOLD (1).ncm` 的原始目录和网易云数据库；确认先按 `trackId/albumId` 匹配两个来源。
- 两首输出均存在，文件名仅冲突组带专辑或稳定 ID 后缀；其他至少 100 首不冲突歌曲的文件名与转换前规则一致。
- 用 ExifTool 逐首检查 Title、Artist、Album、Album Artist、Genre、年份、封面和已有 W4DJ 高级字段；后缀不得出现在任何标签值中。
- 查询 `w4dj.sqlite3`：两首均为独立 `local_files`/`tracks`，专辑和身份字段可区分；重复执行不会覆盖或重复插入。
- 删除/移动其中一个输出后执行失效扫描，只将对应记录标记为失效，不影响另一首。
- 取消或失败时保留已经提交的另一首；错误报告将冲突消歧结果列为成功，不再列为“拒绝覆盖”。

**验收通过标准：**

| 项目 | 必须满足 |
|---|---|
| 保留 | 不同专辑的同名歌曲全部生成独立文件，数量不减少 |
| 命名范围 | 只有发生实际目标路径冲突的歌曲带后缀；非冲突歌曲文件名 100% 不变 |
| 元数据 | 两首输出的完整标签与各自源记录一致，文件名后缀不污染标签 |
| 数据库 | 两首都有独立输出路径、专辑、`trackId/albumId`（可用时）和分析关联 |
| 安全 | 不覆盖不同身份的已有文件；重复运行幂等；取消/错误不删除已成功输出 |
| 诊断 | 预览、历史和错误报告能显示两个最终路径及专辑身份 |
| 回归 | Rust、前端、格式、构建和 `git diff --check` 全部通过 |

未完成以上任一项，不得宣称该功能验收通过。真实网易云数据库或外置磁盘未挂载时，只能报告自动化结果，不能伪造真实数据通过。

## 2026-08-25 实施与验收结果

- [x] 已完成歌曲身份传递：候选保留 `trackId`、`albumId`、专辑和原始来源路径；W4DJ SQLite 通过 `tracks.netease_track_id` 与兼容的 `w4dj_output_identities` 投影保存身份，不改变网易云源库。
- [x] 已完成仅冲突组消歧：同一基础目标路径的不同身份按专辑生成 `[专辑]` 后缀，专辑仍相同才回退稳定 ID/序号；非冲突候选原名保持不变。已有同名输出会被保留，另一首生成消歧路径。
- [x] 已完成元数据与最终路径分离写回；真实验收读取两首输出的 Title、Artist、Album 与源记录一致，文件名后缀没有进入音频标签；重复预览不会覆盖已有输出。
- [x] 已完成预览提示，只有包含消歧候选的预览显示“同名歌曲已按专辑区分，完整元数据保持不变”。
- [x] 真实 `STONE KOLD` 双曲验收通过：来源为 `STONE KOLD - Skybreak,Subten.ncm` 与 `(1).ncm`，数据库身份分别为 `trackId=2707606350/albumId=album272831136/album=STONE KOLD` 和 `trackId=2714172644/albumId=album274773564/album=HALF BLOOD`；两首转换成功、目标路径不同、W4DJ 身份独立登记，已有 `STONE KOLD - Skybreak, Subten.mp3` 重跑时保留。
- [x] 自动化验证通过：前端 Vitest 191/191、根 `cargo test --all`、Tauri `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、Tauri `cargo check`、Vite 构建、TypeScript、Rust fmt 和 `git diff --check` 均通过。
- [ ] 仍未宣称 100 首非冲突文件名的外置批次验收或逐文件 ExifTool 回读；当前挂载素材只覆盖上述两首真实冲突曲目，外置大批次需另行验收。
