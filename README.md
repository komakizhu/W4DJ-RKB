# W4DJ 网易云曲库同步工具

W4DJ 是一个简单的命令行工具，用于同步网易云音乐（Netease Cloud Music）下载目录至自己曲库，且支持将 NCM 格式转换为标准音频格式。

## 功能特点

- 🎵 扫描并同步网易云音乐下载目录
- 🔄 支持 `compat` 和 `lossless` 两种模式
- 📁 支持自定义源目录和目标目录
- ⚡ rayon 多线程处理，快速同步大量文件
- 🚀 Rust 编写，内存占用极低
![w4dj](imgs/w4dj.png)


## 安装

### 1.从源码构建

1. 确保已安装 [Rust 工具链](https://www.rust-lang.org/tools/install)
2. 克隆仓库：
   ```bash
   git clone https://github.com/Slipstream-Max/w4dj.git
   cd w4dj
   ```
3. 构建项目：
   ```bash
   cargo build --release
   ```
4. 可执行文件将位于 `target/release/w4dj`
5. 安装ffmpeg

Windows:
```bash
winget update
winget install "FFmpeg (Essentials Build)"
```
Linux:
```bash
sudo apt update
sudo apt install ffmpeg
```
Mac:
```bash
#先安装homebrew
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" 
brew update
brew install ffmpeg
```

### 2.运行已经编译好的Release
解压后设置config.toml 双击运行。
- 对于Mac若需要启用legacy支持（mp3输出）则需要按照上面的指示安装ffmpeg，
Windows端已经拥有捆绑的ffmpeg。

## 使用方法


### 1. 创建配置文件 `config.toml`

Windows 路径请使用 `/`。

1. 兼容模式
   ```toml
   source = "/path/to/netmusic/songs"       # 网易云音乐下载目录
   destination = "/path/to/music/library"   # 目标音乐库目录
   mode = "compat"                          # 兼容模式：输出 mp3
   ```

2. 无损模式
   ```toml
   source = "/path/to/netmusic/songs"       # 网易云音乐下载目录
   destination = "/path/to/music/library"   # 目标音乐库目录
   mode = "lossless"                        # 无损模式：输出 wav / flac / aiff
   lossless_format = "flac"
   ```


### 2. 运行程序

双击 exe，或指定配置文件路径：

```bash
./w4dj --config /path/to/your/config.toml
```

如果要进入 GUI 入口：

```bash
./w4dj --gui --config /path/to/your/config.toml
```

### 3. 运行桌面窗口开发版

桌面窗口使用 Tauri + Web UI。首次运行前先安装前端依赖：

```bash
cd app
npm install
npm run build
cd ..
cargo build --manifest-path src-tauri/Cargo.toml
```

### 4.程序将自动：
   - 扫描源目录和目标目录
   - 识别新增歌曲
   - 按模式执行转换或复制
   - 显示同步进度和结果



## 致谢

 - [anonymous5l/ncmdump](https://github.com/anonymous5l/ncmdump)
 - [iqiziqi/ncmdump.rs](https://github.com/iqiziqi/ncmdump.rs)

## 免责声明

本工具仅用于个人学习和技术研究目的。请确保您遵守相关法律法规，仅同步您拥有合法使用权的音乐文件。
