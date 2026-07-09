# W4DJ • 如果我是DJ 🎧

[![Release](https://img.shields.io/github/v/release/komakizhu/w4dj?color=crimson)](https://github.com/komakizhu/w4dj/releases)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org)
[![Tauri](https://img.shields.io/badge/tauri-v2-blue.svg)](https://tauri.app)

W4DJ 是一款专为 DJ 和音乐爱好者打造的网易云曲库无损同步与解密工具。它拥有极致美观的 macOS 原生流体毛玻璃（Liquid-Glass）暗色桌面视窗设计，同时支持 Windows 平台。

可一键将网易云下载目录扫描、解密并完美同步至你的硬件曲库中，输出完美兼容 CDJ-350 / XDJ-700 / XDJ-RX / CDJ-2000 Nexus 等专业先锋（Pioneer DJ）硬件设备的标准格式。

![w4dj](imgs/w4dj.png)

---

## ✨ 功能特点

* **🎵 智能扫描与解密**：自动识别 NCM 加密格式并提取封面、歌手及歌曲名称等元数据信息，一键还原。
* **🎛️ 极致美学 GUI**：专为 macOS / Windows 打造的沉浸式红黑渐变流体毛玻璃（Liquid-Glass）界面，支持底部日志抽屉折叠收纳。
* **🔄 双模式输出**：
  * **兼容模式**：最高输出高品质 320kbps MP3 格式，保留基本封面元数据。
  * **无损模式**：支持 24-bit / 48000Hz 采样率的 WAV 或 AIFF 无损格式输出，完美兼容先锋 CDJ 硬件的解码限制。
* **💾 偏好自动记忆**：智能记住你的原始文件夹、输出目录、上次选定的转换模式和音质格式，开箱即用。
* **⚡ 飞速多线程**：后端核心采用 Rust 编写，由 `rayon` 驱动多线程并发，极低内存占用，同步数千首歌仅需数秒。
* **🏷️ 标签与元数据保留**：解密转换的同时自动重写 ID3 / FLAC 元数据标签与专辑封面。

---

## 🚀 安装与运行

### 1. 直接下载运行正式版（推荐）

直接在 GitHub Release 页面下载编译好的安装包：

* **📁 macOS 用户**：进入 [Releases](https://github.com/komakizhu/w4dj/releases) 下载 `W4DJ_0.1.0_x64.dmg`。双击打开并拖拽至 `Applications`（应用程序）文件夹即可。
* **📁 Windows 用户**：进入 [Releases](https://github.com/komakizhu/w4dj/releases) 下载 `W4DJ_0.1.0_x64-setup.exe`。双击运行安装程序即可。

> **⚠️ 注意**：Mac 端如果需要使用 MP3 输出（兼容模式），本地需要装有 ffmpeg。
> ```bash
> brew install ffmpeg
> ```

---

### 🛠️ 2. 从源码自主编译

如果你需要对代码进行二次开发，请确保已安装 [Rust 工具链](https://www.rust-lang.org/tools/install) 和 [Node.js](https://nodejs.org/)。

#### 步骤 A：克隆并配置项目
```bash
git clone https://github.com/komakizhu/w4dj.git
cd w4dj
```

#### 步骤 B：编译前端
```bash
cd app
npm install
npm run build
cd ..
```

#### 步骤 C：开发模式运行桌面视窗
```bash
npx tauri dev
```

#### 步骤 D：打包生产版本客户端
```bash
npx tauri build
```

---

## 🤝 致谢

* [anonymous5l/ncmdump](https://github.com/anonymous5l/ncmdump)
* [iqiziqi/ncmdump.rs](https://github.com/iqiziqi/ncmdump.rs)

---

## ⚖️ 免责声明

本工具仅用于个人学习、技术研究与备份目的。请在使用本工具前确保您拥有音乐文件的合法使用权。作者不对因不当使用该工具而产生的任何法律纠纷承担责任。
