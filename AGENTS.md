# W4DJ RKB 项目长期协作规则

## 项目定位

W4DJ RKB 是一个面向 DJ 音乐库整理的跨平台桌面应用：负责把文件夹或单曲转换为适合 Rekordbox/CDJ 工作流的音频文件，并保留、补全和展示歌曲元数据、封面、歌词及 Essentia 分析结果。应用也包含本地歌曲库索引和网易云音乐本地数据读取能力。

## 技术栈

- Rust 2024：音频扫描、FFmpeg 转换、元数据写入、网易云数据库读取、SQLite 歌曲库、历史和报告。
- Tauri 2：桌面窗口、系统文件选择器、命令和事件桥接。
- TypeScript/Vite/Vitest：界面、教程、任务卡、Dashboard、动画和前端测试。
- FFmpeg sidecar：媒体探测和格式转换。
- `id3`、`metaflac` 及现有 RIFF/AIFF 处理：不同音频容器的标签和封面写入。
- `rusqlite` bundled SQLite：W4DJ 自己的私有歌曲库索引；网易云数据库只读。
- Essentia/Essentia.js：增强模式的基础分析和已接入的分析流程；模型能力不得默认阻塞普通转换。

## 主要目录

- `/Users/mac2/Documents/W4DJ RKB/app/`：前端入口、样式、教程、任务卡、歌曲库 Dashboard 和 Vitest 测试。
- `/Users/mac2/Documents/W4DJ RKB/src/`：共享 Rust 库，包含分析、同步、元数据、历史、网易云恢复、媒体探测、歌曲库和缓存。
- `/Users/mac2/Documents/W4DJ RKB/src-tauri/`：Tauri 桌面壳、命令、系统文件操作、sidecar 和图标。
- `/Users/mac2/Documents/W4DJ RKB/tests/`：Rust 集成测试。
- `/Users/mac2/Documents/W4DJ RKB/docs/`：设计、实施计划、测试包和项目状态文档。
- `/Users/mac2/Documents/W4DJ RKB/scripts/`：测试夹具和本地调试脚本；使用前先确认脚本是否属于当前任务。
- `/Users/mac2/Documents/W4DJ RKB/packaging/macos/`：macOS DMG 中使用的正式 Gatekeeper 修复脚本来源。

## 架构原则

1. 两个任务槽独立工作；来源由拖入或选择内容自动识别为文件夹或单曲，不能用任务 1 锁定任务 2。
2. 普通转换和增强分析解耦。普通模式只做转换；增强模式在不破坏已生成音频的前提下追加分析和元数据。
3. 扫描缓存、增强分析缓存、歌曲库 SQLite 和转换历史是不同数据域，清理其中一个不能误删其他数据或模型。
4. 转换输出必须使用临时文件和安全提交；失败或取消时清理临时结果，不覆盖可用旧文件。
5. 网易云数据只读导入 W4DJ 私有索引库，不修改网易云源数据库。曲目 ID、专辑 ID 仅用于内部匹配，不写入最终音频作为伪用户标签。
6. 元数据按容器能力写入并在适用范围内复读校验；原有有效标签不能被低置信度分析结果静默覆盖。
7. 所有长任务应保留可观察的进度、报告和失败原因；前端状态更新不得通过重建按钮 DOM 破坏现有动画或造成文字闪烁。
8. 不接入外部 Agent、MIRFLEX、CLAP 或 LLM；网易云功能依赖本地数据，不依赖非官方网络 API。

## 关键数据来源

- 用户选择的本地音频目录或单曲。
- 网易云本地 `sqlite_storage.sqlite3` 及其记录中的本地路径、封面路径和元数据。
- 用户指定的网易云音乐目录及其 `meta` 封面目录；具体路径不得硬编码为某一个用户账号。
- 应用数据目录中的 `track-analysis.json`、`scan-cache.json`、转换历史和 W4DJ 私有歌曲库 SQLite。
- 已下载的 Essentia 模型文件。模型缺失或损坏应降级为普通转换/基础流程，而不是阻止普通转换。

## 常用验证和构建命令

在仓库根目录执行：

```bash
cargo test --all
cargo fmt --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --lib --all-features -- -D warnings
cargo clippy --all-targets --all-features -- -A dead_code -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -A dead_code -D warnings

PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app test -- --run
PATH=/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:/Users/mac2/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/fallback:$PATH pnpm --dir app build

cargo tauri build --target aarch64-apple-darwin --bundles app
```

Apple Silicon 本地 App 输出在 `src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`。发布构建和 DMG 流程以 `.github/workflows/release.yml`、`.github/workflows/build-artifacts.yml` 为准；macOS DMG 必须继续包含 App、`packaging/macos/解决文件已损坏问题（双击）.command` 和 Applications 链接/目录。

## Git 与 GitHub 约束

- 默认保留用户已有的脏工作树和未跟踪文件；先检查 `git status`，不要用破坏性 `reset --hard`、`checkout --` 或批量删除。
- 不擅自 push、合并 `main`、创建或覆盖 tag、创建 Release、修改版本号或重写历史。
- 只有用户明确要求定稿/推送/发布时，才执行对应 GitHub 操作；CI 失败时只在用户授权范围内修复当前分支。
- 不从其他版本分支复制源码，不把临时测试文件、桌面脚本副本或用户数据混入发布提交。
- 完成代码任务后先测试、展示 `git status` 和 diff 摘要，等待用户确认。

