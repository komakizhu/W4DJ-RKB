# W4DJ RKB v1.0.0 - 如果我是DJ 🎧

**一句话介绍：一键将网易云等流媒体下载的非标音乐，自动解密、重采样并补全封面，转换为 Pioneer DJ 硬件（如 CDJ-2000/RX3）完美兼容的标准无损音频，以便直接拖入 Rekordbox。**

基于 [Slipstream-Max/w4dj](https://github.com/Slipstream-Max/w4dj) 的首个原生桌面客户端正式版。

![w4dj](imgs/w4dj.png)

---

## 为什么需要这个工具？

**核心痛点：你从网易云或 SoundCloud 下载的歌，Pioneer 硬件很可能根本读不出来。**

| 问题 | 具体表现 |
|------|---------|
| 🔒 网易云 `.ncm` 加密 | 文件被加密锁死，无法直接导入任何 DJ 软件 |
| ⚠️ 格式、采样率不兼容 | 网易云下载曲目通常为flac，SoundCloud 正版下载经常是 **96000Hz**，Pioneer RX3 / CDJ-2000 等硬件直接**读不出来、卡死或跳过** |
| ❓ 元数据丢失 | 转换后歌手名变成「未知艺术家」，封面消失，Rekordbox 曲库一片空白（视觉DJ没封面根本记不住歌名的说） |

**W4DJ RKB 就是为了解决这些问题而生的。** 它把你的音乐统一转换为 **24-bit / 48000Hz**——这是 Pioneer CDJ-2000 / XDJ-RX3 等硬件实测最稳定兼容的规格上限（最早可以支持到CDJ-350 / XDJ-700）。

---

## 它做什么？

```
网易云 / SoundCloud 下载目录
        │
        ▼
   ┌─────────────┐
   │  W4DJ RKB   │  ← 解密 + 降采样至 48kHz + 元数据回写
   └─────────────┘
        │
        ▼
输出目录（标准格式，直接拖入 Rekordbox）
```

### 📤 两种输出模式

| 模式 | 输出格式 | 采样率 | 适用场景 |
|------|---------|--------|---------|
| **兼容模式** | 320kbps MP3 | 标准 | 所有 DJ 设备通用，文件体积小 |
| **无损模式** | WAV 或 AIFF | 24-bit / 48000Hz | 追求音质，**确保 RX3 / CDJ-2000 完美兼容** |

> 💡 **为什么是 48000Hz 而不是更高？**
> Pioneer 的 CDJ-350、XDJ-700、CDJ-2000 系列以及 XDJ-RX3 的 DAC 解码上限就是 48kHz。高于这个采样率的文件（如 SoundCloud 的 96kHz）会导致硬件无法识别、播放卡顿或直接跳过曲目。48kHz 是兼顾音质与兼容性的最优选择。
> 
> ⚠️ **注：** 原文件属于低音质的，本工具并不会进行虚假音质提升。如库中同时混有 128kbps 的 MP3 和无损 FLAC，在选择「无损模式」转换后的结果为：128kbps 依然输出 MP3，而 FLAC 则输出无损 WAV。

### 🏷️ 元数据完整保留
- 解密时自动提取并回写：**歌手名、歌曲名、专辑封面**
- 输出文件的 ID3 / FLAC 标签完整，导入 Rekordbox 后不再显示「未知艺术家」

### ⚡ 性能
- Rust 后端 + rayon 多线程并发，数千首歌同步仅需数秒
- macOS 安装包仅约 24 MB

---

## 支持的音源平台

| 平台 | 支持情况 | 说明 |
|------|---------|------|
| 🎵 网易云音乐 | ✅ 完整支持 | 自动解密 `.ncm` 加密文件 |
| 🎶 SoundCloud | ✅ 支持 | 重采样 96kHz → 48kHz，解决硬件不兼容问题 |

---

## 安装

前往 [Releases](https://github.com/komakizhu/w4dj/releases) 页面下载：

| 平台 | 文件 |
|------|------|
| macOS（Intel / Apple Silicon） | `W4DJ RKB_1.0.0_x64.dmg` |
| Windows（x64） | `W4DJ RKB_1.0.0_x64-setup.exe` |

### ⚠️ 首次运行安全提示

<details>
<summary>🍏 macOS 提示「已损坏，打不开」</summary>

这是 macOS Gatekeeper 对未签名应用的正常拦截，任选一种方式解锁：

**方法一（推荐）**：打开 **系统设置 → 隐私与安全性**，找到「已阻止打开 W4DJ RKB」提示，点击 **仍要打开**。

**方法二**：终端运行：
```bash
xattr -cr /Applications/W4DJ\ RKB.app
```
</details>

<details>
<summary>🔌 Windows 弹出「已保护你的电脑」</summary>

点击 **更多信息** → **仍要运行** 即可。
</details>

---

## 致谢

- [Slipstream-Max/w4dj](https://github.com/Slipstream-Max/w4dj) — 原始命令行同步引擎
- [anonymous5l/ncmdump](https://github.com/anonymous5l/ncmdump) — NCM 解密算法
- [iqiziqi/ncmdump.rs](https://github.com/iqiziqi/ncmdump.rs) — Rust 版 NCM 解密库

## 免责声明

本工具仅用于个人学习和技术研究目的。请确保您遵守相关法律法规，仅同步您拥有合法使用权的音乐文件。
