# W4DJ RKB v1.0.0 - 如果我是DJ 🎧

基于 [Slipstream-Max/w4dj](https://github.com/Slipstream-Max/w4dj) 的首个原生桌面客户端正式版。

W4DJ RKB 解决一个具体问题：**把网易云音乐下载的歌曲，转换成 Pioneer DJ 硬件能直接读取的标准格式，导入 Rekordbox 曲库。**

![w4dj](imgs/w4dj.png)

---

## 它做什么？

```
网易云下载目录（含 NCM 加密文件）
        │
        ▼
   ┌─────────────┐
   │  W4DJ RKB   │  ← 自动解密 + 格式转换 + 元数据回写
   └─────────────┘
        │
        ▼
输出目录（标准音频文件，可直接导入 Rekordbox）
```

### 📥 输入
- 指定网易云音乐的本地下载文件夹作为原始目录
- 自动识别并解密 `.ncm` 加密格式文件

### 📤 输出（二选一）

| 模式 | 输出格式 | 适用场景 |
|------|---------|---------|
| **兼容模式** | 320kbps MP3 | 所有 DJ 设备通用，文件体积小 |
| **无损模式** | WAV 或 AIFF（24-bit / 48kHz） | 追求音质，兼容 CDJ-350 / XDJ-700 及以上硬件 |

### 🏷️ 元数据处理
- 解密时自动提取并回写：歌手名、歌曲名、专辑封面
- 输出文件的 ID3 / FLAC 标签完整保留，导入 Rekordbox 后不会显示「未知艺术家」

---

## 安装

前往 [Releases](https://github.com/komakizhu/w4dj/releases) 页面下载对应平台的安装包：

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

## 技术细节

- **后端**：Rust + rayon 多线程并发，数千首歌同步仅需数秒
- **前端**：Tauri 2 + TypeScript，原生窗口体验
- **体积**：macOS 安装包约 11 MB

---

## 致谢

- [Slipstream-Max/w4dj](https://github.com/Slipstream-Max/w4dj) — 原始命令行同步引擎
- [anonymous5l/ncmdump](https://github.com/anonymous5l/ncmdump) — NCM 解密算法
- [iqiziqi/ncmdump.rs](https://github.com/iqiziqi/ncmdump.rs) — Rust 版 NCM 解密库

## 免责声明

本工具仅用于个人学习和技术研究目的。请确保您遵守相关法律法规，仅同步您拥有合法使用权的音乐文件。
