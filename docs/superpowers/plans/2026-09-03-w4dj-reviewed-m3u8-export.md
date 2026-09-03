# W4DJ Reviewed M3U8 Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在用户完成音频转换后导入 `.w4dj` v2，优先用最近一次转换批次与歌单做标题/歌手 BM25F 一一匹配，不足时以 50 分为门槛从已有曲库补齐，并在复用“扫描后转换”外壳的两栏复核界面中检查、手动选歌或移除当前导出列表中的歌曲后导出 M3U8。

**Architecture:** `.w4dj` 继续保持 format version 2，每首 `netease_track_id` 固定为 JSON `null`；网易云 ID 只能在转换元数据检索期间短暂使用，不进入歌单、输出身份、匹配、恢复或导出。转换成功时在 W4DJ SQLite 为实际输出记录 `batch_id`；歌单导入时绑定最近一个尚未被其它歌单认领的成功批次。匹配器只使用标题与歌手的 BM25F 风格归一化分数：最近批次候选优先完成一一分配，缺口才从全库中选择分数至少 50 的候选；两栏复核界面默认接受有效绑定，未绑定行可手动选择本地文件，用户可把不需要的行标记为仅对当前导出列表生效的排除项，导出只消费剩余有效绑定。

## 2026-09-03 UI follow-up

最终 UI 语义以本节为准：界面不显示逐行“确认此行”，有效自动匹配默认接受；匹配来源只显示“最近转换”“已有曲库”或“手动选择”，分数显示为“匹配度 N%”。左栏复选框只代表选择要移除的行，支持单首删除、批量删除、全选和恢复。`excluded` 只改变该歌单的当前导出列表，不删除 W4DJ SQLite 曲库记录、不删除本地音频，也不改变 `.w4dj` v2；后端导出前过滤排除行并对剩余行执行完整路径校验。

**Tech Stack:** Rust 2024、Rusqlite、Serde、Tauri 2、TypeScript、Vite、Vitest、现有 W4DJ `w4dj.sqlite3` 与转换历史。

## Global Constraints

- 产品版本保持 `3.2.3`，不修改 SemVer。
- `.w4dj` 继续使用 `format_version: 2`；不得升级为 v3。
- 新导出和重新导出的每个轨道必须包含 `"netease_track_id": null`；不得写 `""`、`"null"`、`0` 或推测 ID。
- 网易云歌曲 ID 只允许在转换阶段查询网易云本地数据库；不得用于歌单解析、持久化、匹配、排序、去重、输出身份、sidecar、M3U8 或 UI。
- 用户必须先完成转换，再导入 `.w4dj`；歌单导入不启动转换、不扫描网易云数据库。
- “新增歌曲”定义为最近一次成功安全提交且记录了同一 `batch_id` 的输出，包括新建和覆盖更新；跳过且没有重新提交的旧输出不属于该批次。
- 歌单有 N 首时，不写死“7 首”：最近批次候选少于 N 才从已有曲库补齐。
- 只有已有曲库回退使用 `score >= 50` 门槛；最近批次候选按一一分配结果展示，即使低于 50 也不丢弃。
- 自动匹配不得把同一实际输出分配给两首不同歌曲；只有歌单中规范化标题与歌手完全相同的重复位置允许复用同一 `track_key`。
- 导出前必须展示两栏复核 UI；有效绑定默认接受，复选框只表示“选择该行从当前导出列表移除”。
- M3U8 必须包含当前导出列表中未排除行的原始位置顺序；保留行未绑定或路径不可用时禁用导出，不静默省略保留行。
- 手动选择文件后立即成为该行的有效绑定；不需要额外确认，用户仍可单独或批量移除该行。
- 隐藏恢复数据只允许由显式 W4DJ 导入、匹配、手动绑定或导出操作写入；普通转换不写 sidecar。
- 保留用户已有音频、数据库、转换历史、分析缓存和脏工作树；不执行 commit、push、merge 或 release。

---

## 已冻结的验收数据集（实施前完成）

以下数据已在 2026-09-03 使用 `dj-crate-digger` 生成并完成网页来源核验，不得在最终验收时临时换歌：

- `test-artifacts/w4dj/acceptance-tech-house-8.w4dj`
- `test-artifacts/w4dj/acceptance-uk-garage-10.w4dj`
- `test-artifacts/w4dj/acceptance-melodic-techno-6.w4dj`
- `test-artifacts/w4dj/acceptance-expected-results.json`

这些文件由 `/test-artifacts/w4dj/` Git 忽略规则隔离，只用于本机测试，不得 stage、commit 或发布。三份 `.w4dj` 均已通过以下合同：`format == "w4dj"`、`format_version == 2`、position 从 1 连续递增、每首只有 `position/title/artist_display/netease_track_id` 四个字段、`netease_track_id` 严格为 JSON `null`。

来源页已经直接核对：

- Tech House 前 7 首：[Tech House 2025, Vol.1](https://digitalempirerecords.bandcamp.com/album/tech-house-2025-vol-1)；第 8 首：[Toolroom Miami 2025](https://toolroom.bandcamp.com/album/toolroom-miami-2025)。
- UK Garage 10 首：[FABRICLIVE SELECTS VII](https://fabriclive.bandcamp.com/album/fabriclive-selects-vii)。
- Melodic Techno 6 首：[Steyoyoke Perception Vol. 11](https://steyoyoke.bandcamp.com/album/steyoyoke-perception-vol-11)。

### 数据集 A：用户提供的 John Summit 回归样本（8 首）

原 `.w4dj` 不改写。它是 v2，轨道内不提供 `netease_track_id`；解析器必须把“字段缺失”和“字段为 null”都视为没有 ID，且匹配行为完全相同。静音源固定写入以下 title/artist 标签，以重现括号、`[Extended]`、歌手分隔符和地区后缀差异：

| 位置 | `.w4dj` 标题 — 歌手 | 静音源标签 title — artist | 预期 |
|---:|---|---|---|
| 1 | Ferrari Extended Mix — James Hype, Miggy Dela Rosa | Ferrari (Extended Mix) — James Hype / Miggy Dela Rosa | recent 自动匹配 |
| 2 | Eat Your Man (with Nelly Furtado) Extended Mix — Dom Dolla, Nelly Furtado | Eat Your Man (with Nelly Furtado) [Extended] — Dom Dolla / Nelly Furtado | recent 自动匹配 |
| 3 | Sun Goes Down Extended Mix — Cloonee | Sun Goes Down (Extended Mix) — Cloonee | recent 自动匹配 |
| 4 | Gimme That Bounce Original Mix — Mau P | Gimme That Bounce (Original Mix) — Mau P | recent 自动匹配 |
| 5 | Atmosphere Extended Mix — FISHER (OZ), Kita Alexander | Atmosphere (Extended Mix) — FISHER / Kita Alexander | recent 自动匹配 |
| 6 | Taka Extended Mix — SIDEPIECE, San Pacho | Taka (Extended Mix) — SIDEPIECE / San Pacho | recent 自动匹配 |
| 7 | Voodoo Extended Mix — Gorgon City | Voodoo (Extended Mix) — Gorgon City | recent 自动匹配 |
| 8 | Where You Are Extended Mix — John Summit, HAYLA | Where You Are (Extended Mix) — John Summit / HAYLA | recent 自动匹配 |

固定结果：`recent=8`、`library=0`、`matched=8`、`reviewRows=8`、人工确认后 `m3u8Entries=8`、`omitted=0`；每个自动匹配的分数必须位于 `65..=100`，且不得读取或比较网易云 ID。

### 数据集 B：Tech House 归一化样本（8 首）

| 位置 | `.w4dj` 标题 — 歌手 | 静音源标签 title — artist | 预期 |
|---:|---|---|---|
| 1 | Deeper MSTR C (Extended Mix) — MicahelBM, JAYIE | Deeper MSTR C Extended Mix — MicahelBM / JAYIE | recent 自动匹配 |
| 2 | Trago de Ron (Original Mix) — Marc Suarez | Trago de Ron Original Mix — Marc Suarez | recent 自动匹配 |
| 3 | Bounce Back Original Mix) — Trizzoh | Bounce Back (Original Mix) — Trizzoh | recent 自动匹配 |
| 4 | ONLYFANS (Extended Mix) — S_Zer0, Valmonte | ONLYFANS Extended Mix — S Zer0 & Valmonte | recent 自动匹配 |
| 5 | The Way (Original Mix) — MXJ, AJSE | The Way Original Mix — MXJ / AJSE | recent 自动匹配 |
| 6 | Like This (Extended Mix) — Diseptix, Incognet, Alex Helder | Like This Extended Mix — Diseptix / Incognet / Alex Helder | recent 自动匹配 |
| 7 | Paralyzed (Original Mix) — CYRUS | Paralyzed Original Mix — Cyrus | recent 自动匹配 |
| 8 | Wrong Feels Right (Extended Mix) — Format:B | Wrong Feels Right Extended Mix — Format B | recent 自动匹配 |

固定结果：`recent=8`、`library=0`、`matched=8`、`reviewRows=8`、人工确认后 `m3u8Entries=8`、`omitted=0`；分数位于 `65..=100`。

### 数据集 C：UK Garage 冗余候选样本（歌单 10 首，最近批次 12 首）

| 位置 | `.w4dj` 标题 — 歌手 | 静音源标签 title — artist | 预期 |
|---:|---|---|---|
| 1 | Lose My Cool — DJ Q | Lose My Cool — DJ Q | recent 自动匹配 |
| 2 | Hyper — Bodhi | Hyper — Bodhi | recent 自动匹配 |
| 3 | This Bassline Smells Like Oil — Ghoulish | This Bassline Smells Like Oil — Ghoulish | recent 自动匹配 |
| 4 | Best Of Me — 1111 | Best of Me — 1111 | recent 自动匹配 |
| 5 | Target — Gemi, Kori | Target — Gemi / Kori | recent 自动匹配 |
| 6 | Dub Selecta 16 — Daffy, PJ Bridger | Dub Selecta 16 — PJ Bridger & Daffy | recent 自动匹配，歌手顺序不敏感 |
| 7 | Riddim — Eloquin, Reimond | Riddim — Eloquin / Reimond | recent 自动匹配 |
| 8 | ON TOUR — SEMPA | On Tour — Sempa | recent 自动匹配 |
| 9 | The Power — TARZI | The Power — TARZI | recent 自动匹配 |
| 10 | Up a Little — Me & George | Up A Little — Me and George | recent 自动匹配 |
| — | 无 | Target Practice — Kori | 冗余，不得导出 |
| — | 无 | The Power Within — TARZAN | 冗余，不得导出 |

固定结果：`recent=12`、`library=0`、`matched=10`、`excluded=2`、`reviewRows=10`、人工确认后 `m3u8Entries=10`、`omitted=0`；一一分配必须选择 10 个目标候选，两个相似标题干扰项不得抢占位置。

### 数据集 D：Melodic Techno 历史补齐与手动恢复样本（6 首）

| 位置 | `.w4dj` 标题 — 歌手 | 静音源标签 title — artist | 批次 | 预期 |
|---:|---|---|---|---|
| 1 | Ipnosi (Original Mix) — RIVE | Ipnosi Original Mix — RIVE | 历史 | library 自动补齐 |
| 2 | Rhea (Original Mix) — ANRA (UA) | Rhea Original Mix — ANRA | 历史 | library 自动补齐 |
| 3 | Solara (Original Mix) — Hakan (NL) | Solara Original Mix — Hakan | 最近 | recent 自动匹配 |
| 4 | Vespertine (Original Mix) — Salbah | Vespertine Original Mix — Salbah | 最近 | recent 自动匹配 |
| 5 | Calling (Black Sharp Remix) — René Diehl | Calling Black Sharp Remix — Rene Diehl | 最近 | recent 自动匹配 |
| 6 | Nlreb Mra Alrrih (Playing With The Wind) — Sahalé | Nlreb Mra Alrrih Playing With The Wind — Sahale | 最近 | recent 自动匹配 |

固定结果：`recent=4`、`library=2`、`matched=6`、两个 library 匹配均 `score >= 50`、`reviewRows=6`、人工确认后 `m3u8Entries=6`、`omitted=0`。

同一歌单另跑手动恢复变体：把位置 1 唯一的历史候选替换为 `Untitled Fixture — Unknown Artist`，自动阶段必须得到 `matched=5` 且位置 1 右栏为空；手动选择正确静音文件后该行变为 `manual`、分数 100、`confirmed=false`，用户重新勾选后才能导出 6/6。

### 冻结的最终验收矩阵

| 场景 | 歌单 | 最近批次 | 历史补齐 | 手动 | 复核行 | M3U8 | 关键断言 |
|---|---:|---:|---:|---:|---:|---:|---|
| A John Summit 回归 | 8 | 8 | 0 | 0 | 8 | 8 | `[Extended]`、括号、双歌手和 `(OZ)` 不丢歌 |
| B Tech House 归一化 | 8 | 8 | 0 | 0 | 8 | 8 | 标点、大小写、下划线和三歌手均可匹配 |
| C UKG 两个冗余 | 10 | 12 | 0 | 0 | 10 | 10 | 选对 10 首，排除 2 个相似干扰项 |
| D Melodic 历史补齐 | 6 | 4 | 2 | 0 | 6 | 6 | 全库只接受分数至少 50 的补齐 |
| E 手动恢复 | 6 | 4 | 1 | 1 | 6 | 6 | 低于 50 留空；手选后强制重新确认 |

所有场景都必须满足：两栏横向逐行对应；初始确认数为 0；N/N 之前导出按钮禁用；N/N 后导出的每个路径存在、非空、可读取；顺序与 position 完全一致；`neteaseIdUsesOutsideConversion=0`。

---

## File Map

- `src/dj_playlist.rs`：维持 v2 wire compatibility，内部轨道模型不携带网易云 ID，序列化固定写 `null`。
- `src/dj_playlist_match.rs`：标题/歌手分词、BM25F 风格评分、最近批次优先与全库补齐的一一分配。
- `src/w4dj_library.rs`：schema 迁移、转换批次 provenance、歌单批次认领、匹配/确认/手动绑定持久化和恢复 sidecar。
- `src-tauri/src/main.rs`：转换提交时传入 batch ID；歌单命令不加载网易云 resolver；增加手动路径绑定与逐行确认命令。
- `src/m3u8.rs`：只消费完整且已确认的绑定，保持原始位置顺序。
- `app/src/dj-playlist.ts`：v2 前端类型继续接受 `neteaseTrackId: null`，不把它用于逻辑。
- `app/src/dj-playlist-review.ts`：纯函数渲染两栏复核内容和计算是否允许导出。
- `app/src/app.ts`：复核状态、文件选择、打开文件、确认和导出编排。
- `app/src/styles.css`：复用 preview modal 外壳，补充两栏行布局和窄屏降级。
- `tests/dj_playlist.rs`、`tests/dj_playlist_match.rs`、`tests/w4dj_library.rs`、`tests/m3u8.rs`：Rust 合同与回归。
- `app/src/dj-playlist.test.ts`、`app/src/dj-playlist-review.test.ts`、`app/src/app.test.ts`：前端合同、两栏 UI 和完整导出流程。
- `scripts/generate-w4dj-silence-fixtures.sh`：从 `.w4dj` 生成带标题/歌手标签的短静音 WAV 测试源，不包含原曲音频。
- `tests/w4dj_playlist_acceptance.rs`：调用实际普通转换与歌单匹配/导出代码完成端到端验收。

---

### Task 1: 固定 `.w4dj` v2 的无歌曲 ID 边界

**Files:**
- Modify: `src/dj_playlist.rs`
- Modify: `src-tauri/src/main.rs:4355-4795`
- Modify: `app/src/dj-playlist.ts`
- Test: `tests/dj_playlist.rs`
- Test: `tests/w4dj_library.rs`
- Test: `app/src/dj-playlist.test.ts`

**Interfaces:**
- Consumes: 现有 `.w4dj` v2 顶层字段与 `position/title/artist_display/netease_track_id` wire 字段。
- Produces: `ImportedDjPlaylistTrack` 仅包含位置、标题、歌手及派生文本；`MinimalW4djTrack` 始终将 `netease_track_id` 序列化为 `None`，生成 JSON `null`。

- [x] **Step 1: 写失败测试，锁定 v2、输入 ID 忽略和输出 null**

```rust
#[test]
fn v2_track_id_is_accepted_for_compatibility_but_never_enters_the_domain_model() {
    let playlist = parse_w4dj_playlist(
        br#"{"format":"w4dj","format_version":2,"export_id":"p1","playlist":{"name":"Set"},"tracks":[{"position":1,"title":"Song","artist_display":"Artist","netease_track_id":"42"}]}"#,
        None,
    ).unwrap();
    assert_eq!(playlist.tracks[0].title, "Song");
    let exported = serde_json::from_slice::<serde_json::Value>(
        &serialize_w4dj_playlist(&playlist).unwrap(),
    ).unwrap();
    assert_eq!(exported["format_version"], 2);
    assert!(exported["tracks"][0]["netease_track_id"].is_null());
}
```

- [x] **Step 2: 运行测试并确认旧行为失败**

Run: `cargo test --test dj_playlist v2_track_id_is_accepted_for_compatibility_but_never_enters_the_domain_model -- --exact`

Expected: FAIL，因为当前内部模型保留并重新导出输入 ID。

- [x] **Step 3: 从领域模型删除歌曲 ID，但在 wire 层兼容读取并固定输出 null**

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTrack {
    position: u64,
    title: String,
    artist_display: String,
    #[serde(default, rename = "netease_track_id")]
    _ignored_netease_track_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct MinimalW4djTrack<'a> {
    position: u64,
    title: &'a str,
    artist_display: &'a str,
    netease_track_id: Option<&'a str>,
}
```

构造 `MinimalW4djTrack` 时固定 `netease_track_id: None`。从 `ImportedDjPlaylistTrack` 删除 `netease_track_id` 和 `set_netease_track_id`；`dedupe_key` 只由 `title + artist_display + position` 派生。

- [x] **Step 4: 删除歌单和输出身份的网易云 ID 补全调用**

从 `import_w4dj_playlist`、`load_imported_dj_playlist`、`export_imported_dj_playlist_w4dj`、`match_imported_dj_playlist`、`export_imported_dj_playlist_m3u8` 删除 `load_netease_resolver_for_identity`、`enrich_imported_dj_playlist_ids` 和 `enrich_output_identities_from_netease` 调用。删除 `W4djLibrary::enrich_imported_dj_playlist_ids`；保留旧 SQLite 列仅为非破坏迁移兼容，但所有新写入必须为 NULL，所有匹配查询不得读取该列。

- [x] **Step 5: 阻止转换检索 ID越过转换边界**

转换阶段仍可用网易云记录获取标题、歌手、专辑、封面和歌词；从 `upsert_committed_output`、`upsert_committed_output_in_root`、`upsert_lightweight_output_inner` 和所有调用方删除 track/album ID 参数。停止写入并停止查询 `w4dj_output_identities`，保留旧表仅为了非破坏升级。`OutputIdentityManifestEntry` 删除 `netease_track_id`/`netease_album_id`，旧 sidecar 中同名字段由 Serde 忽略且不得迁移到新记录。

- [x] **Step 6: 运行聚焦测试**

Run: `cargo test --test dj_playlist && cargo test --test w4dj_library && PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run dj-playlist.test.ts`

Expected: 全部 PASS；测试中不存在按歌曲 ID 匹配或补全的正向断言。

---

### Task 2: 为安全提交的输出记录转换批次并在导入时认领最近批次

**Files:**
- Modify: `src/w4dj_library.rs:33-60,300-430,1550-1610`
- Modify: `src-tauri/src/main.rs:8490-8585,8750-9350,9850-9975`
- Test: `tests/w4dj_library.rs`
- Test: `tests/task_state.rs`

**Interfaces:**
- Produces: `CommittedOutputFacts::conversion_batch_id: Option<String>`，以及数据库生成的 `committed_at_ms`。
- Produces: `W4djLibrary::claim_latest_conversion_batch(playlist_id: &str) -> W4djResult<Option<String>>`。
- Produces: `W4djLibrary::dj_output_candidates_for_batch(batch_id: &str) -> W4djResult<Vec<DjOutputCandidate>>`。
- Consumes: 每个确认转换任务已有的 `job.batch_id`。

- [x] **Step 1: 写 schema 与查询失败测试**

```rust
#[test]
fn playlist_claims_the_latest_unclaimed_committed_batch() {
    let directory = tempfile::tempdir().unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    register_test_output(&mut library, directory.path(), "batch-old", "old.mp3", "Old", "Artist");
    register_test_output(&mut library, directory.path(), "batch-new", "new.mp3", "New", "Artist");
    library.upsert_imported_dj_playlist(&test_playlist("playlist-a")).unwrap();
    assert_eq!(library.claim_latest_conversion_batch("playlist-a").unwrap().as_deref(), Some("batch-new"));
    library.upsert_imported_dj_playlist(&test_playlist("playlist-b")).unwrap();
    assert_eq!(library.claim_latest_conversion_batch("playlist-b").unwrap(), None);
}
```

- [x] **Step 2: 运行测试并确认接口尚不存在**

Run: `cargo test --test w4dj_library playlist_claims_the_latest_unclaimed_committed_batch -- --exact`

Expected: 编译失败，缺少批次字段与认领接口。

- [x] **Step 3: 增加非破坏 schema 迁移**

将 `W4DJ_SCHEMA_VERSION` 从 3 升到 4，只升级内部数据库 schema，不改变产品版本或 `.w4dj` 格式。增加：

```sql
ALTER TABLE w4dj_track_meta ADD COLUMN conversion_batch_id TEXT;
ALTER TABLE w4dj_track_meta ADD COLUMN committed_at_ms INTEGER;
ALTER TABLE imported_dj_playlists ADD COLUMN claimed_batch_id TEXT;
CREATE INDEX IF NOT EXISTS w4dj_track_meta_batch
ON w4dj_track_meta(conversion_batch_id, committed_at_ms);
CREATE INDEX IF NOT EXISTS imported_dj_playlists_claimed_batch
ON imported_dj_playlists(claimed_batch_id);
```

迁移必须通过 `pragma_table_info` 判断列是否存在，重复打开数据库不得失败。

- [x] **Step 4: 在安全提交事务中记录 batch ID**

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommittedOutputFacts {
    pub source_size_bytes: Option<u64>,
    pub source_modified_at_ms: Option<u64>,
    pub conversion_mode: Option<String>,
    pub lossless_format: Option<String>,
    pub filename_rule: Option<String>,
    pub netease_filename_format: Option<String>,
    pub filename_normalization_policy: Option<String>,
    pub conversion_batch_id: Option<String>,
}
```

`run_confirmed_sync_task` 和 direct conversion 注册路径都把当前 `batch_id.clone()` 写入该字段；只有最终输出安全提交成功后才能同时更新 `w4dj_track_meta.conversion_batch_id` 与 `committed_at_ms=now_ms()`。扫描、预览、分析更新、失败、取消和跳过不得改变这两个提交字段。

- [x] **Step 5: 原子认领最近的未认领批次**

`claim_latest_conversion_batch` 在事务内先按 `MAX(committed_at_ms)` 取得绝对最新的非空 `conversion_batch_id`。如果这个最新批次已经被其它歌单认领，则返回 `None`，不得倒退认领更老且可能无关的批次；后续直接走全库回退。未被认领时写入当前歌单。重新导入相同 `playlist_id` 时保留原认领，除非原批次已不存在。

- [x] **Step 6: 运行批次回归**

Run: `cargo test --test w4dj_library playlist_claims_the_latest_unclaimed_committed_batch -- --exact && cargo test --test task_state`

Expected: PASS；失败或跳过的文件不出现在 `dj_output_candidates_for_batch`。

---

### Task 3: 实现仅标题/歌手的 BM25F 风格评分与确定性一一分配

**Files:**
- Modify: `src/dj_playlist_match.rs`
- Test: `tests/dj_playlist_match.rs`

**Interfaces:**
- Produces: `pub fn bm25f_track_score(query_title: &str, query_artist: &str, candidate_title: &str, candidate_artist: &str, corpus: &[DjOutputCandidate]) -> u8`，范围 `0..=100`。
- Produces: `pub fn match_imported_playlist_with_priority(playlist: &ImportedDjPlaylist, recent: &[DjOutputCandidate], library: &[DjOutputCandidate]) -> DjPlaylistMatchReport`。
- Match methods: `recentBm25f`、`libraryBm25f`、`manual`。

- [x] **Step 1: 写评分与分配失败测试**

```rust
#[test]
fn bm25f_partial_title_and_artist_prefers_the_intended_track() {
    let corpus = vec![
        candidate("right", "Where You Are Extended Mix", "John Summit / HAYLA"),
        candidate("wrong", "Where Are You Now", "Lost Frequencies"),
    ];
    let right = bm25f_track_score("Where You Are (Extended Mix)", "John Summit, HAYLA", &corpus[0].title, &corpus[0].artist_display, &corpus);
    let wrong = bm25f_track_score("Where You Are (Extended Mix)", "John Summit, HAYLA", &corpus[1].title, &corpus[1].artist_display, &corpus);
    assert!(right > wrong);
    assert!(right >= 50);
}

#[test]
fn recent_batch_is_assigned_first_and_library_only_fills_the_gap_above_fifty() {
    let report = match_imported_playlist_with_priority(
        &playlist([track(1, "A", "One"), track(2, "B", "Two")]),
        &[candidate("recent-a", "A", "One")],
        &[candidate("library-b", "B Remix", "Two"), candidate("weak", "Noise", "Else")],
    );
    assert_eq!(report.matched_count, 2);
    assert_eq!(report.matches[0].match_method.as_deref(), Some("recentBm25f"));
    assert_eq!(report.matches[1].match_method.as_deref(), Some("libraryBm25f"));
    assert!(report.matches[1].score.unwrap() >= 50);
}
```

- [x] **Step 2: 运行测试并确认新接口尚不存在**

Run: `cargo test --test dj_playlist_match`

Expected: 编译失败，缺少 BM25F 接口。

- [x] **Step 3: 实现音乐文本 token 化**

复用 `normalize_identity_text` 的大小写、全半角和标点处理。每个规范化单词生成完整 token，并为长度至少 4 的 token生成字符 trigram；歌手先按 `,，、;；&+/× feat ft featuring with` 分隔，再生成 token。不要把 `extended`、`original`、`remix`、`radio`、`edit`、`live`、`mix` 当停用词。

- [x] **Step 4: 实现可解释的 0–100 BM25F 归一化**

对标题和歌手分别计算 corpus IDF 与 BM25 饱和项，参数固定为 `k1=1.2`、`b=0.75`。每个字段用“实际 term contribution / 同一 query term 的理论最大 contribution”归一化到 `0..=1`，再计算：

```rust
let combined = 0.65 * title_score + 0.35 * artist_score;
let score = (combined * 100.0).round().clamp(0.0, 100.0) as u8;
```

这样 50 分是稳定的绝对门槛，不得用“本行最高候选归一化为 100”的相对算法。

- [x] **Step 5: 实现最近批次优先的一一分配**

先生成所有“歌单位置 × recent 候选”边，按 `score DESC → title component DESC → artist component DESC → position ASC → track_key ASC` 排序，并依次占用尚未分配的位置和候选。recent 数量等于 N 时必须分配 N 个不同输出；recent 少于 N 时全部 recent 参与分配。随后只为未分配位置生成全库候选边，过滤 `score < 50` 和已占用输出，再按相同顺序分配。

重复位置只有在规范化标题与歌手均相同时才允许复用同一输出；其它位置保持一对一。结果不得包含或比较网易云 ID。

- [x] **Step 6: 增加 8 对 10、6 对 8、同分和重复歌曲测试**

覆盖：8 首歌单/10 首 recent 选出最优 8 首；6 首 recent/8 首歌单从全库补 2 首；49 分候选保持空白；50 分候选允许补齐；同分按 track key 稳定；重复歌单位置可复用；不同歌曲不可复用。

- [x] **Step 7: 运行匹配器全量测试**

Run: `cargo test --test dj_playlist_match`

Expected: 全部 PASS；删除 `netease_id_is_the_first_and_only_identity_when_present` 和 `an_id_miss_does_not_fall_back_to_a_lookalike_title` 这类旧合同，替换为“ID 完全不影响结果”的回归测试。

---

### Task 4: 持久化自动绑定、手动本地文件和逐行确认

**Files:**
- Modify: `src/w4dj_library.rs:350-430,829-1135`
- Modify: `src-tauri/src/main.rs:4660-4810`
- Test: `tests/w4dj_library.rs`
- Test: `tests/m3u8.rs`

**Interfaces:**
- Produces: `set_imported_dj_playlist_match_by_path(playlist_id: &str, position: u64, path: &Path) -> W4djResult<DjPlaylistMatchReport>`。
- Produces: `set_imported_dj_playlist_match_confirmed(playlist_id: &str, position: u64, confirmed: bool) -> W4djResult<DjPlaylistMatchReport>`。
- `DjPlaylistTrackMatch` 新增 `destination_path: Option<PathBuf>`、`confirmed: bool`、`candidate_source: Option<String>`。

- [x] **Step 1: 写持久化失败测试**

```rust
#[test]
fn changing_a_manual_file_clears_confirmation_until_the_user_checks_again() {
    let mut library = prepared_playlist_library();
    library.set_imported_dj_playlist_match_confirmed("p1", 1, true).unwrap();
    library.set_imported_dj_playlist_match_by_path("p1", 1, &second_audio()).unwrap();
    let row = &library.get_imported_dj_playlist_match_report("p1").unwrap().matches[0];
    assert_eq!(row.destination_path.as_deref(), Some(second_audio().as_path()));
    assert!(!row.confirmed);
    assert_eq!(row.match_method.as_deref(), Some("manual"));
}
```

- [x] **Step 2: 增加确认字段迁移**

为 `imported_dj_playlist_matches` 增加 `confirmed INTEGER NOT NULL DEFAULT 0` 和 `candidate_source TEXT`。任何自动重新匹配、文件失效或手动更换都写 `confirmed=0`；只有显式确认命令可以写 1。

- [x] **Step 3: 让匹配查询返回当前实际路径**

通过 `track_key` 连接 `w4dj_track_meta.destination_path`，只返回存在于索引的当前路径。路径消失时把该行改成 `missing`、清除确认并保留歌单左栏信息。

- [x] **Step 4: 实现手动路径绑定命令**

Tauri 命令先 canonicalize 绝对路径，拒绝目录、符号链接、零字节和非支持音频扩展。若路径已经存在于 `w4dj_track_meta`，直接使用其 `track_key`；否则通过现有媒体读取能力取得标题/歌手，并以该文件父目录为显式手动 root 调用 `upsert_output_file` 后取得 `track_key`。然后保存 `match_method='manual'`、`score=100`、`confirmed=0`。

- [x] **Step 5: 删除歌曲 ID决定的重复复用规则**

`set_imported_dj_playlist_match` 不再比较 ID。只有另一个位置的规范化标题和歌手与当前位置完全相同，才允许共用 `track_key`；否则返回“该本地歌曲已分配给歌单中的另一首歌”。

- [x] **Step 6: 让显式 W4DJ 操作保存无 ID恢复绑定**

扩展现有隐藏 manifest，为已确认绑定保存 `playlistId`、`position`、`relativePath`、`title`、`artistDisplay`、`score` 和 `matchMethod`；不保存网易云 ID。普通转换仍不写 manifest。恢复仅在 W4DJ 导入/匹配/导出边界执行，路径安全检查继续拒绝绝对路径与 `..`。

- [x] **Step 7: 运行持久化和 M3U8 前置测试**

Run: `cargo test --test w4dj_library && cargo test --test m3u8`

Expected: 全部 PASS；更换文件后未确认、路径失效后未确认、清库后通过显式 W4DJ 操作恢复已确认绑定。

---

### Task 5: 建立与“扫描后转换”一致的两栏逐首复核 UI

**Files:**
- Create: `app/src/dj-playlist-review.ts`
- Create: `app/src/dj-playlist-review.test.ts`
- Modify: `app/src/app.ts:350-425,950-1010,2013-2090,2348-2460,4620-4955,6180-6305`
- Modify: `app/src/styles.css:2396-2710,3360-3535`
- Test: `app/src/app.test.ts`

**Interfaces:**
- Produces: `renderDjPlaylistReview(report, lang, busy): string`。
- Produces: `canExportReviewedPlaylist(report): boolean`。
- Services: `setImportedDjPlaylistMatchByPath(playlistId, position, path)`、`setImportedDjPlaylistMatchConfirmed(playlistId, position, confirmed)`。

- [x] **Step 1: 写纯 UI 失败测试**

```ts
it('renders exactly two data columns and leaves an unmatched local cell selectable', () => {
  const html = renderDjPlaylistReview(reportWithOneMissing(), 'zh', false);
  const root = document.createElement('div');
  root.innerHTML = html;
  expect(root.querySelectorAll('[data-role="dj-review-heading"]')).toHaveLength(2);
  expect(root.querySelector('[data-role="dj-review-left"]')?.textContent).toContain('Atmosphere');
  expect(root.querySelector('[data-role="dj-review-right"]')?.textContent).toContain('选择本地歌曲');
  expect(canExportReviewedPlaylist(reportWithOneMissing())).toBe(false);
});
```

- [x] **Step 2: 复用 preview modal 外壳**

复核窗口使用 `.preview-modal`、`.preview-dialog`、`.preview-head`、`.preview-actions`，保持与“扫描后转换”相同的宽度、圆角、阴影、间距、滚动和底部按钮。不要嵌套歌曲卡片，不增加第三个数据栏。

- [x] **Step 3: 渲染固定两栏行**

```html
<div class="dj-review-grid" role="table">
  <div class="dj-review-heading" data-role="dj-review-heading">歌单歌曲</div>
  <div class="dj-review-heading" data-role="dj-review-heading">本地歌曲</div>
  <div class="dj-review-cell" data-role="dj-review-left">01 Where You Are — John Summit, HAYLA</div>
  <div class="dj-review-cell" data-role="dj-review-right">Where You Are.mp3 · 92%</div>
</div>
```

复选框放在左侧单元格内部，因此视觉上仍只有两栏。左栏只显示序号、歌单标题和歌手。右栏有匹配时显示可点击文件名、歌手、浅色分数和“更换”；无匹配时内容留空，仅显示“选择本地歌曲”。不展示展开候选列表或多层诊断卡。

- [x] **Step 4: 接入逐行确认语义**

自动匹配与手动选择默认均未勾选。点击左栏复选框调用确认命令；右栏文件被更换或失效时复选框自动取消。底部显示 `已确认 X/N`；`canExportReviewedPlaylist` 仅在每行 `trackKey`、`destinationPath` 和 `confirmed` 都有效时返回 true。

- [x] **Step 5: 接入打开和选择本地歌曲**

点击文件名复用 `openDestinationFile`，且不得将窗口带到前台。点击“选择本地歌曲/更换”调用 Tauri dialog：

```ts
const selected = await open({
  multiple: false,
  directory: false,
  filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'aiff', 'aif', 'flac', 'm4a'] }],
});
```

选择取消不改变当前行；成功后调用 `setImportedDjPlaylistMatchByPath` 并重绘。

- [x] **Step 6: 移除部分导出和旧拦截文案**

删除“仅导出已匹配”“另有 N 首未导出”“歌曲都无法匹配到输出文件”等流程。选择最近歌单后即进入复核 UI；没有自动匹配时展示 N 行空右栏，允许逐首手动选择。全部确认后才进入现有“复制音频/仅引用音频”选择。

- [x] **Step 7: 增加窄屏行为**

桌面保持两栏同一横轴逐行对应；`max-width: 720px` 时每一行仍作为一个整体上下堆叠左/右单元格，不允许把所有左栏与所有右栏拆成两个独立列表。路径使用省略号但保留 title/aria-label。

- [x] **Step 8: 运行前端聚焦测试**

Run: `PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run dj-playlist-review.test.ts app.test.ts`

Expected: PASS；断言复核窗口复用 preview shell、恰有两栏、空右栏可选择、文件可打开、更换后取消确认、N/N 前不能导出。

---

### Task 6: 只从完整且已确认的绑定导出 M3U8

**Files:**
- Modify: `src/m3u8.rs`
- Modify: `src-tauri/src/main.rs:4755-4935`
- Modify: `app/src/app.ts:4830-4955,6180-6240`
- Test: `tests/m3u8.rs`
- Test: `app/src/app.test.ts`

**Interfaces:**
- Consumes: 持久化的 `DjPlaylistMatchReport`，每行必须 `status='matched'`、`confirmed=true` 且实际路径可读。
- Produces: 原始歌单位置顺序的完整 N 行 M3U8；不接受 `allow_partial`。

- [x] **Step 1: 写完整导出失败测试**

```rust
#[test]
fn reviewed_export_rejects_any_unconfirmed_row_and_never_omits_tracks() {
    let report = report_with_confirmed_rows(vec![true, false]);
    let error = resolve_reviewed_playlist(&report).unwrap_err();
    assert!(error.to_string().contains("第 2 首尚未确认"));
}
```

- [x] **Step 2: 移除 partial 参数和 omitted 成功语义**

Tauri 导出命令不再接收 `allow_partial`。导出前读取已保存报告，不重新运行匹配；逐行验证确认状态、`track_key` 和当前可读路径。任一行失败时返回精确位置错误且不创建 M3U8/复制目录。

- [x] **Step 3: 保持原始顺序和原子导出**

按 `position ASC` 生成全部 N 个 `#EXTINF + relative path` 条目。复制模式先把 N 个音频全部复制并复读验证，再原子提交 M3U8；任何复制失败清理本次新建文件，不覆盖旧可用导出。

- [x] **Step 4: 前端只在 N/N 已确认时触发导出**

导出按钮 disabled 条件与 `canExportReviewedPlaylist` 完全一致。导出后显示 `已导出 N/N`，不得显示 omitted 数量；复核绑定保持持久化，用户可再次导出或更换其中一行。

- [x] **Step 5: 运行导出测试**

Run: `cargo test --test m3u8 && PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run app.test.ts`

Expected: PASS；覆盖引用模式、复制模式、路径失效、未确认、顺序、重复位置和双输出根目录。

---

### Task 7: 使用预生成数据执行全量验证、诊断报告和人工验收

**Files:**
- Create: `scripts/generate-w4dj-silence-fixtures.sh`
- Create: `tests/w4dj_playlist_acceptance.rs`
- Modify: `docs/project-state.md`
- Modify: `docs/handoff.md`
- Verify: `docs/superpowers/plans/2026-09-03-w4dj-reviewed-m3u8-export.md`

**Interfaces:**
- Consumes: Tasks 1–6 的最终实现。
- Produces: 真实 8 首样本、DJ 技能补充样本、短静音实际转换、两栏确认和完整 M3U8 的可复现证据；以及最新 arm64 App 和不含网易云 ID 匹配数据的 full runtime report。

- [x] **Step 1: 校验用户提供的 8 首主验收样本**

主样本使用原文件，不根据记忆重建：

`/Users/mac2/Library/Messages/Attachments/d6/06/F9E72671-D45C-4DCA-94FE-EE5AE9A0AD6E/john-summit-edm-house-set.w4dj`

验收前执行：

```bash
jq -e '
  .format == "w4dj" and
  .format_version == 2 and
  (.tracks | length) == 8 and
  all(.tracks[]; .netease_track_id == null)
' '/Users/mac2/Library/Messages/Attachments/d6/06/F9E72671-D45C-4DCA-94FE-EE5AE9A0AD6E/john-summit-edm-house-set.w4dj'
```

Expected: `true`，退出码 0。这里 `.netease_track_id == null` 同时兼容字段缺失和 JSON null；两种情况都只表示“无 ID”，不得触发 ID 查询或推断。八首固定为 Ferrari、Eat Your Man、Sun Goes Down、Gimme That Bounce、Atmosphere、Taka、Voodoo、Where You Are，位置 1–8 不变。

- [x] **Step 2: 用 DJ Crate Digger 技能真实生成并冻结三组补充 `.w4dj`**

本步骤已在实施开始前完成，最终验收只消费固定文件，不再联网选歌或重新生成：

1. 8 首 Tech House：刻意包含合作歌手、`Extended Mix`、`Original Mix` 和括号。
2. 10 首 UK Garage：刻意包含多歌手、`feat.`、短标题和相似标题。
3. 6 首 Melodic Techno：刻意包含 Remix 名称、非 ASCII 艺人名和不同标点。

三份文件写入技能规定的本地隔离目录：

```text
test-artifacts/w4dj/acceptance-tech-house-8.w4dj
test-artifacts/w4dj/acceptance-uk-garage-10.w4dj
test-artifacts/w4dj/acceptance-melodic-techno-6.w4dj
```

三份文件及 `acceptance-expected-results.json` 已通过 JSON 语法和对应合同检查，并已由 `.gitignore` 隔离。验收时若任一文件内容与本计划“已冻结的验收数据集”不同，必须直接失败，不得静默接受漂移。

- [x] **Step 3: 创建可重复的短静音音频生成器**

`scripts/generate-w4dj-silence-fixtures.sh` 接收 `.w4dj` 和输出目录，用 `jq` 逐行读取位置、标题和歌手，再调用 FFmpeg 生成 2 秒 stereo PCM WAV：

```bash
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i 'anullsrc=r=44100:cl=stereo' -t 2 \
  -metadata "title=$track_title" \
  -metadata "artist=$track_artist" \
  -c:a pcm_s16le "$fixture_output"
```

脚本必须使用 `mktemp -d` 或显式传入的 `/private/tmp/w4dj-playlist-acceptance-*` 目录；不得写用户音乐目录。文件名使用零填充位置和安全化标题，标签保留 `.w4dj` 原始标题/歌手。生成后逐个用 `ffprobe` 验证时长大于 0、title 非空、artist 非空。

- [x] **Step 4: 用实际 W4DJ 普通转换代码转换静音源**

`tests/w4dj_playlist_acceptance.rs` 不伪造输出数据库行：它把 Step 3 生成的 WAV 作为真实输入，调用与桌面普通“扫描后转换”相同的 preview、FFmpeg 转换、安全提交和 `upsert_committed_output_in_root` 路径，输出到新的临时目录。禁用增强分析，不要求网易云数据库，不读取用户真实曲库。

Run:

```bash
W4DJ_ACCEPTANCE_PLAYLIST='/Users/mac2/Library/Messages/Attachments/d6/06/F9E72671-D45C-4DCA-94FE-EE5AE9A0AD6E/john-summit-edm-house-set.w4dj' \
cargo test --test w4dj_playlist_acceptance real_eight_track_playlist_converts_matches_and_exports -- --ignored --exact --nocapture
```

Expected: 生成 8 个非空实际转换输出；每个输出都记录同一 acceptance batch ID；没有读取或写入网易云歌曲 ID。

- [x] **Step 5: 主样本必须通过完整识别与 M3U8 验收**

在转换完成后才导入主样本。断言：认领 Step 4 的 batch；BM25F 生成 8 个 `recentBm25f` 一一绑定；复核模型恰有 8 行且左右两栏位置一一对应；逐行模拟打开路径、勾选确认；最终导出恰好 8 个 M3U8 音频条目。每条相对路径必须解析为存在、非空、可打开的实际转换文件，顺序与 `.w4dj` position 1–8 完全一致。

该场景只有以下条件全部成立才算通过：

```text
playlist=8
converted=8
matched=8
reviewRows=8
confirmed=8
m3u8Entries=8
readableEntries=8
omitted=0
neteaseIdUsesOutsideConversion=0
```

- [x] **Step 6: 按冻结矩阵覆盖冗余候选、缺口补齐和手动选歌**

对三份 DJ 技能产物分别生成静音 WAV 并执行实际转换：

- Tech House：严格使用数据集 B 的 8 组静音源标签，要求 8/8 recent 自动绑定并完整导出。
- UK Garage：严格使用数据集 C 的 10 组目标标签，再生成 `Target Practice — Kori` 与 `The Power Within — TARZAN` 两个冗余静音文件；最近批次共 12 个候选，要求选出目标 10 首，两个干扰项不进入 M3U8。
- Melodic Techno：严格使用数据集 D；先转换位置 1–2 形成历史曲库，再用新 batch 转换位置 3–6；导入后要求 4 首来自 recent、2 首从全库以 `score >= 50` 补齐。

另复制一个候选并故意把 title/artist 改成低于 50 分，断言右栏保持空白；随后通过系统文件选择流程手动绑定一个本地静音音频，更换后确认状态为 false，重新勾选后才允许导出 6/6。

- [x] **Step 7: 运行两栏 UI 自动验收**

Vitest 必须检查真实主样本映射后的 DOM：恰好两个表头；8 个左栏和 8 个右栏；文件名按钮带实际路径；空右栏有“选择本地歌曲”；复选框均未默认确认；全部 8 行确认前导出按钮 disabled，确认后 enabled；更换任一行后再次 disabled。

Run: `PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run dj-playlist-review.test.ts app.test.ts`

Expected: 全部 PASS。

- [x] **Step 8: 运行格式与静态检查**

Run: `cargo fmt --all -- --check && cargo clippy --lib --all-features -- -D warnings && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -A dead_code -D warnings`

Expected: 全部退出码 0。

- [x] **Step 9: 运行 Rust 与前端全量测试**

Run: `cargo test --all`

Run: `PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run`

Expected: 全部 PASS。

- [x] **Step 10: 运行构建检查**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Run: `PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app build`

Expected: 全部退出码 0。

- [x] **Step 11: 编译 Apple Silicon App**

Run: `cargo tauri build --target aarch64-apple-darwin --bundles app`

Expected: 生成 `src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`，版本为 3.2.3。

- [x] **Step 12: 对照冻结矩阵汇总隔离验收结果**

逐项输出数据集 A–E 的实际 `recent/library/manual/matched/reviewRows/confirmed/m3u8Entries/omitted`，并与本计划顶部矩阵逐格比较。任何计数不同、任何目标曲目缺失、任何冗余曲目进入 M3U8、任何未确认行被导出，都算验收失败。

- [x] **Step 13: 核对 full runtime report 数据边界**

报告应包含歌单位置、标题、歌手、BM25F 分数、候选来源、确认状态、track key、实际路径和转换 batch ID；不得包含或使用歌单/输出网易云歌曲 ID。SQLite 快照可保留旧兼容列，但报告必须说明它们未参与本次流程。

- [x] **Step 14: 展示工作树状态并停止**

Run: `git status --short --branch && git diff --stat && git diff --check`

Expected: `git diff --check` 退出码 0；只报告本任务修改，不提交、不推送。

---

## Acceptance Criteria

1. `.w4dj` 保持 v2，每首 `netease_track_id` 严格为 JSON `null`；旧 v2 非空 ID 可读取但被忽略，重新导出为 null。
2. 网易云 ID只在转换元数据查询中使用，不进入任何歌单、输出身份、sidecar、匹配、UI、报告或 M3U8 流程。
3. 用户完成转换后再导入歌单；导入自动认领最近一个未认领的成功转换批次。
4. 8 首歌单＋8 首最近输出得到 8 个一一对应结果；8 首＋10 首只选择最佳 8 首。
5. 最近输出少于 N 时，系统从已有曲库补充 BM25F 分数至少 50 的候选；低于 50 保持右栏空白供手动选择。
6. BM25F 只使用标题 65% 与歌手 35%，支持部分 token 和字符 trigram 匹配，输出稳定的 0–100 分数。
7. 自动结果通过确定性全局排序分配，不因输入顺序或 HashMap 顺序变化。
8. 复核 UI 复用“扫描后转换”的 modal shell，主体只有“歌单歌曲 / 本地歌曲”两栏，逐行横向对应。
9. 每行可点击打开本地文件，也可通过系统文件选择器更换；空右栏始终可手动选择。
10. 复选框代表用户已检查；更换或失效会取消确认，只有 N/N 有路径且 N/N 已确认才允许导出。
11. M3U8 始终按歌单位置生成完整 N 行，不再提供部分导出或静默 omitted 成功。
12. 普通转换不写隐藏清单；显式 W4DJ 操作可以保存和恢复不含网易云 ID 的绑定。
13. Rust、Tauri、前端测试、格式、Clippy、Vite 和 arm64 App 构建全部通过，工作树不自动提交。
14. 用户提供的 `john-summit-edm-house-set.w4dj` 必须用 8 个短静音源经过实际普通转换后达到 `8 converted / 8 matched / 8 confirmed / 8 M3U8 entries / 0 omitted`。
15. DJ Crate Digger 技能生成的三组本地 `.w4dj` 必须分别覆盖等量 recent、recent 带冗余、recent 不足后从历史曲库补齐和低于 50 分后手动选歌。
16. 静音夹具只证明转换、元数据、匹配、复核和导出管线，不宣称验证真实音频内容或网易云下载正确性。

## Non-goals

- 不让 W4DJ 在导入歌单后自动开始下载或转换。
- 不调用网易云在线 API，不修改网易云源数据库。
- 不引入音频哈希、声纹、模型或 LLM 匹配。
- 不增加第三栏、候选展开卡片或复杂匹配诊断层级。
- 不允许未确认项目被静默省略，也不通过复制错误文件来伪造完整结果。
- 不升级 `.w4dj` format version，不改变产品版本 3.2.3。
