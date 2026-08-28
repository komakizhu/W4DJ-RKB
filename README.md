# W4DJ RKB

[English version](README.en.md)

## 中文

面向 DJ 音乐库的本地整理、转换与音频分析工具。

![W4DJ RKB 主程序界面](imgs/w4dj.png)

W4DJ RKB 基于并继续复刻开发自 [Slipstream-Max/w4dj](https://github.com/Slipstream-Max/w4dj)，保留原项目的本地扫描、增量同步和音频转换方向。

## 1. 产品主要功能

- **音频转换与同步**：支持 NCM、MP3、FLAC、WAV 等音频格式；将下载音乐清洗并转换为 Rekordbox 与 CDJ/XDJ 支持的线下播放格式。提供两个可独立设置来源和输出目录的插槽并支持拖入歌曲文件夹或单曲。
- **一键导入 DJ Set**：导入[老炮DJ](https://github.com/komakizhu/dj-crate-digger-skill)生成的 `.w4dj` 歌单，生成二维码一键导入网易云歌单；DJ Set 经过 W4DJ 转换之后，导出 `.m3u8` 供 Rekordbox 使用。详见[具体操作指南](https://github.com/komakizhu/dj-crate-digger/blob/main/docs/w4dj/README.md)。
- **元数据与封面**：尽量保留或恢复标题、艺术家、专辑、曲号、Genre、歌词和封面；网易云数据库只读，输出歌曲由独立的 `w4dj.sqlite3` 管理。
- **界面与教程**：支持中文/英文和深色/浅色模式切换，并可随时重新打开使用教程。
- **安全与可恢复**：支持跳过、覆盖和仅更新元数据；文件名规则会处理非法字符和冲突；输出先写临时文件并安全提交；转换历史、运行会话和手动错误报告分开保存。
- **桌面体验**：macOS 原生 Tauri 界面，支持可视化进度；FFmpeg、分析模型和用户数据均在本地处理。
- **歌曲库 Dashboard（开发中）**：集中查看已经转换的歌曲，了解哪些歌曲可以正常使用、哪些文件已经失效，并支持搜索、筛选、排序和批量清理。
- **增强音频分析（开发中）**：自动为歌曲补充速度、调性、响度、能量、可舞性、Genre 和情绪等信息，并支持查看进度、取消和继续分析。

## 2. 如何从老炮DJ Skill 导入歌单、准备网易云歌曲并导出 Rekordbox 播放列表

[老炮DJ](https://github.com/komakizhu/dj-crate-digger-skill)（DJ Crate Digger）负责选歌并生成 `.w4dj` 歌单文件，W4DJ RKB 接收这个文件，继续完成本地歌曲准备、转换和播放列表导出。`.w4dj` 只保存歌单名称、曲目顺序、官方歌名和艺人信息，不包含音频文件。

完整流程是：[老炮DJ](https://github.com/komakizhu/dj-crate-digger-skill)生成 `.w4dj` → 导入 W4DJ RKB → 在网易云音乐创建歌单并下载歌曲 → W4DJ RKB 转换音频并导出 `.m3u8` → 导入 Rekordbox。

[老炮DJ](https://github.com/komakizhu/dj-crate-digger-skill) 与 W4DJ RKB 的交接教程见[详细教程](https://github.com/komakizhu/dj-crate-digger/blob/main/docs/w4dj/README.md)。

W4DJ RKB 不会替用户下载或获取版权音频；歌曲必须先通过合法渠道准备到本地。W4DJ RKB 的歌曲库以实际输出文件和 `w4dj.sqlite3` 为准。

## 3. 许可证与项目来源

W4DJ RKB 自己编写的代码采用 **GNU AGPL-3.0-only**，完整条款见 [`LICENSE`](LICENSE)。该许可只适用于本项目原创代码；上游复刻部分、第三方依赖、随包模型和其他资源仍按各自原许可证执行。发布或再分发时，请同时遵守本项目及各依赖、模型的适用条款。

- **复刻与上游来源**：[Slipstream-Max/w4dj](https://github.com/Slipstream-Max/w4dj)。W4DJ RKB 保留该项目的来源说明，不替上游项目重新授予许可证。
- **NCM 解密相关**：[anonymous5l/ncmdump](https://github.com/anonymous5l/ncmdump)、[iqiziqi/ncmdump.rs](https://github.com/iqiziqi/ncmdump.rs)。
- **Essentia.js**：AGPL-3.0，[许可证文本](https://www.gnu.org/licenses/agpl-3.0.html)。
- **TensorFlow.js**：Apache-2.0，[许可证文本](https://www.apache.org/licenses/LICENSE-2.0)。
- **随包音频分析模型**：CC BY-NC-SA 4.0，[许可证文本](https://creativecommons.org/licenses/by-nc-sa/4.0/)。模型归属、转换说明和商业授权信息见 [`src-tauri/resources/essentia-models/NOTICE.md`](src-tauri/resources/essentia-models/NOTICE.md)。

本项目仅用于处理用户合法拥有或有权处理的音频。商业分发前，请分别确认 Essentia.js、预训练模型及其他依赖的授权范围。
