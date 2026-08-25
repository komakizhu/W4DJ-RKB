# W4DJ Minimal Playlist Format and ID-based Matching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** 将 .w4dj 歌单收敛为只包含歌单顺序、显示身份和网易云歌曲 ID 的最小交换格式，并让 M3U8/歌单匹配优先使用 ID，不再依赖不可靠文件名。

**Architecture:** 新导出的 .w4dj 使用全新的 v2 最小格式，只写协议字段、歌单名称和每首歌曲的 position/title/artist_display/netease_track_id。解析器严格拒绝 v1 及任何旧字段，不做旧文件迁移。导入后首先通过 w4dj.sqlite3 的网易云身份映射查找实际输出路径；只有没有 ID 时才执行唯一的标题+歌手回退匹配，歧义结果不得自动选择。

**Tech Stack:** Rust/Serde/Rusqlite/Tauri、TypeScript/Vitest、W4DJ w4dj.sqlite3、相对路径 M3U8。

## Global Constraints

- 产品版本保持 3.2.0-beta.3，不修改 SemVer。
- 新导出格式只包含必要字段；旧版 v1 .w4dj 明确拒绝，不做兼容迁移。
- 新格式版本固定为 2；format_version 不是可选字段。
- netease_track_id 始终以 JSON 字符串保存，禁止转为 JSON 数字。
- 网易云 SQLite 始终只读；Dashboard、匹配和 M3U8 的权威歌曲来源仍是 w4dj.sqlite3。
- 匹配成功后使用数据库中的实际 destination_path，不根据歌单文件名重新定位或重命名音频。
- 同一网易云歌曲在歌单中重复出现时保留每个 position，不能因为 ID 相同而删除第二个播放位置。
- 没有 ID 且标题+歌手无法唯一匹配时返回待确认结果，不静默猜测。
- 不删除用户音频、w4dj.sqlite3、旧 .w4dj 文件或 track-analysis.json。
- 不 commit、push、merge、release；不修改版本号。

---

### Task 1: 定义 v2 最小格式并严格拒绝旧版

**Files:**
- Modify: src/dj_playlist.rs
- Modify: app/src/dj-playlist.ts
- Test: tests/dj_playlist.rs

**New exported wire shape:**

~~~json
{
  "format": "w4dj",
  "format_version": 2,
  "export_id": "playlist-001",
  "playlist": { "name": "UK Bass" },
  "tracks": [
    {
      "position": 1,
      "title": "Midnight Request Line",
      "artist_display": "Skream",
      "netease_track_id": "123456789012345678"
    }
  ]
}
~~~

**Internal contract:**

~~~rust
pub struct ImportedDjPlaylistTrack {
    pub position: u64,
    pub title: String,
    pub artist_display: String,
    pub netease_track_id: Option<String>,
    pub dedupe_key: String,
    pub netease_import_line: String,
}
~~~

- [x] 写解析测试：只含 v2 协议字段、playlist.name 和最小歌曲字段的文件可以导入。
- [x] 写拒绝测试：format_version=1、platform_refs、dedupe_key、duration、旧版额外字段和未知字段均明确失败，不做迁移。
- [x] 让 v2 轨道只接受 position/title/artist_display/netease_track_id；duration、BPM、调性等字段不进入解析模型。
- [x] 让 dedupe_key 由程序内部生成：有 ID 时使用 netease:<id>，无 ID 时使用规范化标题+歌手键；文件不携带它。
- [x] 更新 TypeScript 类型，加入 neteaseTrackId: string | null，不保留旧 platformRefs 类型。
- [x] 运行 cargo test --test dj_playlist 和前端对应 Vitest 测试。

### Task 2: 收敛 .w4dj 导出字段并保留网易云 ID

**Files:**
- Modify: src/dj_playlist.rs
- Modify: src-tauri/src/main.rs
- Modify: app/src/dj-playlist.ts
- Test: tests/dj_playlist.rs
- Test: app/src/dj-playlist.test.ts

新导出的每首歌曲只允许写 position/title/artist_display/netease_track_id；顶层只写 format/format_version=2/export_id/playlist.name/tracks。没有网易云 ID 时省略 netease_track_id，不写 "unknown" 占位字符串。

- [x] 增加独立的 MinimalW4djExport/MinimalW4djTrack 序列化结构，并将 format_version 固定为 2。
- [x] 导出时从已持久化的 W4DJ 歌单读取网易云身份；ID 以字符串写出，没有 ID 时保留标题和歌手。
- [x] 禁止写出 duration、bpm、musical_key、album_or_ep、expected_filename_hint、URL、状态和本地路径。
- [x] 增加 JSON 断言，确认新导出不包含旧字段，并确认大整数 ID 原样保留。
- [x] 运行 cargo test --test dj_playlist 和前端导出测试。

### Task 3: 以 w4dj.sqlite3 网易云 ID 作为 M3U8 唯一匹配主键

**Files:**
- Modify: src/w4dj_library.rs
- Modify: src/dj_playlist_match.rs
- Modify: src/m3u8.rs
- Modify: src-tauri/src/main.rs
- Test: tests/dj_playlist_match.rs
- Test: tests/m3u8.rs

**Interfaces:**

~~~rust
pub enum DjPlaylistMatchKind {
    NeteaseTrackId,
    UniqueTitleArtistFallback,
    Ambiguous,
    Unmatched,
}

pub struct DjPlaylistMatch {
    pub position: u64,
    pub track_id: Option<String>,
    pub destination_path: Option<PathBuf>,
    pub kind: DjPlaylistMatchKind,
    pub candidates: Vec<PathBuf>,
}
~~~

- [x] 在 W4DJ 查询层增加按 netease_track_id 查询，只返回 status=available 且文件存在可读的 destination_path。
- [x] 改变匹配优先级：ID 精确命中直接使用 destination_path；不查询网易云 SQL，不用文件名猜测覆盖精确命中。
- [x] 没有 ID 时使用规范化 Title+Artist；0 个结果为 Unmatched，1 个结果为 UniqueTitleArtistFallback，多个结果为 Ambiguous。
- [x] M3U8 只消费已确认路径；Ambiguous/Unmatched 项不写入，并在摘要中列出 position、标题、歌手和候选路径。
- [x] 同一 ID 出现在多个 position 时生成多个 M3U8 行，不按 track ID 去重。
- [x] 覆盖 ID 命中、ID 不存在、唯一回退、歧义回退、重复播放位置和失效文件测试。
- [x] 运行 Rust 全量测试覆盖歌单匹配和 M3U8 测试目标。

### Task 4: 转换与 W4DJ 库身份写入闭环

**Files:**
- Modify: src/w4dj_library.rs
- Modify: src/sync.rs
- Modify: src/preview.rs
- Modify: src-tauri/src/main.rs
- Test: tests/sync_policy.rs
- Test: tests/preview.rs

- [x] 转换成功后的 netease_track_id 与实际 destination_path 一起登记到 w4dj.sqlite3，不能只存源路径。
- [x] ID 只用于身份匹配，不触发已存在输出重命名；Task 1 的数据库命名策略仍只在新目标路径生成前执行。
- [x] 普通本地歌曲允许身份为空，歌单对其只能走标题+歌手回退。
- [x] 用临时输出目录验证预览、转换、ID 登记、标签写回和源文件不变。
- [x] 运行 Rust 全量测试覆盖 preview 和 sync_policy 测试目标。

### Task 5: UI 导入诊断与旧文件拒绝

**Files:**
- Modify: app/src/dj-playlist.ts
- Modify: app/src/app.ts
- Modify: app/src/styles.css
- Modify: src-tauri/src/main.rs
- Test: app/src/app.test.ts
- Test: app/src/dj-playlist.test.ts

- [x] 每首歌显示“网易云 ID 精确匹配 / 标题歌手回退 / 未匹配 / 需要确认”。
- [x] ID 作为文本展示和复制内容，不转浮点、不截断。
- [x] 歧义结果必须手动选择候选路径后才能加入 M3U8，取消选择保持未匹配。
- [x] 旧版文件明确拒绝。错误提示须指出需要重新导出 v2，不尝试从 platform_refs、dedupe_key 或 duration:"unknown" 自动迁移。
- [x] 网易云 ID 以字符串形式展示；错误的数字 ID 不做浮点转换或自动迁移。
- [x] 运行前端全量 Vitest，覆盖 app 和 dj-playlist 测试。

### Task 6: 文档、真实验收与交接

**Files:**
- Modify: 计划.md
- Modify: docs/project-state.md
- Modify: docs/handoff.md
- Create: docs/superpowers/plans/2026-08-25-minimal-w4dj-playlist-format.md
- Create: /private/tmp/W4DJ-minimal-playlist-format-handoff.md

- [x] 记录 v1 拒绝规则、新 v2 最小格式、ID 匹配优先级和未执行人工验收。
- [x] 只读核对 uk-bass-simulated-10.w4dj 为 v1/旧字段；生产解析器测试确认 UnsupportedVersion，新导出序列化测试确认不产生 duration 字段。
- [x] 通过 ID 精确匹配、ID 不同候选和重复 position 回归夹具验证 destination_path 选择与顺序保持。
- [x] 对重复播放歌曲验证 M3U8 保留两个 position。
- [x] 逐项比较 v2 ID、w4dj.sqlite3 身份映射、实际输出路径和 M3U8 行的受控夹具，不依赖文件名相等。
- [x] 运行 cargo test --all、Tauri 测试、前端 Vitest、Vite、fmt、Tauri check、Tauri Clippy 和 git diff --check。
- [x] 编译 arm64 App，验证 Mach-O 为 arm64、版本仍为 3.2.0-beta.3，并提供绝对路径链接。
- [ ] 记录手机扫码、在线搜索、Rekordbox 和未挂载外置卷等环境限制。

## Acceptance Criteria

1. 新导出的 .w4dj 只包含协议字段、歌单名称和歌曲 position/title/artist_display/netease_track_id，不包含 duration、BPM、调性、专辑、文件名提示、URL、状态或本地路径。
2. netease_track_id 作为字符串完整保留，至少覆盖 18 位 ID，不发生精度变化。
3. 只有 position/title/artist_display 加协议字段时，歌单能够导入；没有网易云 ID 只影响精确匹配，不阻断导入。
4. 旧版 v1 .w4dj 不被导入，解析器返回明确的 UnsupportedVersion/格式错误，不做隐式迁移。
5. M3U8 匹配优先使用 w4dj.sqlite3 的网易云 ID；文件名不同、标题相似或输出路径改变都不影响精确命中。
6. 标题+歌手只有唯一候选时才允许回退；0 个或多个候选必须显示未匹配/需确认。
7. 同一歌曲重复出现在不同 position 时，M3U8 保留重复播放行和原始顺序。
8. 分析、封面、元数据写回不会改变已登记 destination_path，也不会覆盖有效标签。
9. 网易云数据库保持只读，Dashboard 和 M3U8 不把网易云 SQL 或旧 library-dashboard.sqlite3 作为歌曲来源。
10. Rust、前端、格式、Tauri 构建和 arm64 App 验证通过；所有环境限制明确记录。

## Non-goals

- 不增加新的歌曲目录偏好、网易云在线搜索、封面下载策略或高级分析模型。
- 不批量重命名既有输出文件。
- 不删除旧 .w4dj 文件，不删除 track-analysis.json，不改变产品版本号；旧文件只会被拒绝读取。
