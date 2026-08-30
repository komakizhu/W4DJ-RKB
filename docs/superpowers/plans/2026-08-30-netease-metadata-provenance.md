# 网易云元数据来源隔离与持久化键修复

队列标记：`side-plan:01a05072-netease-metadata-provenance-v1-confirmed-2026-08-30`

## 目标

落实“原文件有效值优先 → 网易云正式数据库字段补全 → 可靠文件名推断兜底”，并把轻量 locator 的持久化匹配键与运行时宽松比较键隔离。轻量缓存只保留定位所需的 Track ID、来源定位和最小匹配键；正式 Title/Artist/Album 只有在最终匹配后重新读取网易云原始行才能进入写回链路。

## 已实施

- 持久化键仅执行首尾 trim 与 lowercase；标点、内部空格、全半角及弯引号保持。旧缓存 schema `1` 自动视为 stale，当前 schema 为 `2`。
- 宽松标点/空白折叠只在内存比较中使用；Track ID 优先，无 ID 的同分多候选拒绝猜测。
- locator 与正式 `NeteaseTrackIdentity` 分离，locator 不再伪造正式文本；最终 resolver 按 locator 从只读数据库重新读取原始字段。
- 元数据诊断按字段记录 `sourceTag`、`neteaseDatabase`、`filenameInference`，并记录 `exact`、`caseOnly`、`whitespaceOnly`、`punctuationOnly`、`different` 差异类别；Title/Artist/Album 均参与输出复读校验。
- W4DJ 输出登记使用最终容器内实际复读的文本，不以候选匹配键或文件名静默补写；复读为空/登记失败时保留音频并记录逐曲失败。

## 验收记录

新增持久化键、旧 schema、逐字段来源/专辑补全、正式输出复读和缺失标签拒绝回归。根库 `cargo test --all`、Tauri `cargo test --manifest-path src-tauri/Cargo.toml`、前端 Vitest、TypeScript、Vite、fmt、check、Tauri 严格 Clippy 及根库允许既有 dead-code 的 all-targets Clippy 均通过。根库不抑制既有 dead-code 的严格 all-targets Clippy 仍会因历史未使用 API 失败，未将该既有诊断伪装为通过。

只读审计使用应用数据目录中的 544 条 locator 和网易云只读数据库：544/544 行可回读，旧缓存标记为 `schemaVersion=1`（按新代码会判 stale 并重建）；按新持久化键规则重新计算，224 个字段级键会保留原始内部空格/标点，涉及 207 个 Track ID。审计未写入任一数据库、缓存或音频。

最终验证结果为：根库 129 个单元测试、各集成测试全部通过；Tauri 74 个测试通过；前端 12 个文件/209 项通过；TypeScript、Vite、cargo check、cargo fmt 和 `git diff --check` 通过。最新 arm64 App 已在最终代码上重建，版本保持 `3.2.0-beta.3`。真实用户音频仅做只读审计，未执行写回。

## 约束

不修改网易云数据库、用户音频或版本号，不新增 baseline/hash/冻结 contract/release gate，不 commit、push、merge 或 release。真实数据验收只读或使用合法临时音频副本。
