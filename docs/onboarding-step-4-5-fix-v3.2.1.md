---
title: W4DJ RKB 3.2.1 使用引导第 4、5 步修复说明
source_version: 3.2.0
target_version: 3.2.1
edition: formal
status: implemented
---

# W4DJ RKB 3.2.1 使用引导第 4、5 步修复说明

## 交接结论

本次修改基于正式版 `3.2.0` 源码完成，目标版本为 `3.2.1`。不要从 Legacy 版本分支复制本次实现，也不要使用 `src-tauri/tauri.legacy.conf.json` 构建正式版。

第 5 步说明卡片现在与“教程”按钮处于同一个 `tutorial-anchor` 容器中，使用相对定位显示在按钮正下方。教程按钮以后移动、增加同级功能按钮或发生响应式布局变化时，说明卡片会跟随按钮，不再依赖固定的页面坐标。

## 问题背景

使用引导共有 5 步：

1. 选择输出模式
2. 选择来源
3. 选择输出目录
4. 开始转换
5. 重新查看教程

原问题有两部分：

- 第 4 步说明卡片位置过低，可能遮挡底部“同时开始”按钮。
- 第 5 步说明的是右上角“教程”按钮，但卡片使用固定的 `top/right` 坐标。教程按钮位置变化后，卡片无法保持在按钮正下方。

## 实现规则

### 第 4 步

- 桌面布局：说明卡片距离底部 `236px`，避开“同时开始”按钮。
- 窄窗口布局：说明卡片距离底部 `190px`，同时保留左右边距。

### 第 5 步

- 教程按钮包裹在 `.tutorial-anchor` 中。
- 第 5 步说明卡片作为教程按钮的同级子元素渲染在该容器内。
- `.tutorial-anchor` 使用 `position: relative`。
- 说明卡片使用 `top: calc(100% + 16px)`，因此始终位于教程按钮容器的正下方。
- 窄窗口使用 `top: calc(100% + 12px)`，并以视口宽度限制卡片宽度。
- 第 5 步定位不能重新改为固定的 `top: 88px`、`right: 24px` 或其他页面绝对坐标。

## 涉及文件

| 文件 | 修改内容 |
| --- | --- |
| `app/src/app.ts` | 增加教程按钮锚点容器；将第 5 步卡片放入该容器；拆分背景层和说明卡片渲染。 |
| `app/src/styles.css` | 增加 `.tutorial-anchor` 相对定位，以及第 5 步卡片在按钮下方的响应式定位。 |
| `app/src/app.test.ts` | 增加回归断言，确认第 5 步卡片和教程按钮属于同一锚点容器。 |
| `CHANGELOG.md` | 记录第 4、5 步引导修复。 |
| `Cargo.toml`、`src-tauri/Cargo.toml` | 版本升级到 `3.2.1`。 |
| `app/package.json`、`app/package-lock.json` | 前端版本升级到 `3.2.1`。 |
| `src-tauri/tauri.conf.json` | 正式 Tauri 配置版本升级到 `3.2.1`。 |
| `Cargo.lock`、`src-tauri/Cargo.lock`、`src/config.rs` | 同步锁文件和命令行版本号。 |

## 后续维护要求

如果未来“教程”按钮移动到其他布局区域，保留以下关系：

```html
<div class="tutorial-anchor" data-onboarding-anchor="tutorial">
  <button data-action="open-help" data-onboarding-target="tutorial">
    教程
  </button>
  <section class="onboarding-callout" data-role="onboarding-modal">
    ...
  </section>
</div>
```

可以移动整个 `.tutorial-anchor`，也可以调整教程按钮在容器中的布局，但不要把第 5 步说明卡片移回独立的全屏弹层并用页面坐标定位。若重构顶部栏，必须继续保证：

1. 教程按钮和第 5 步卡片共用同一个定位容器。
2. 卡片的垂直起点使用按钮容器高度加间距。
3. `data-onboarding-target="tutorial"` 仍只出现在当前被高亮的教程按钮上。
4. 引导遮罩层仍位于卡片下方，卡片和高亮按钮保持可见、可操作。

## 验证结果

已在正式版 `main` 源码上完成验证：

- 前端完整测试：12 个测试文件，220 项通过。
- 第 `app/src/app.test.ts` 测试文件：145 项通过。
- Vite 生产构建：通过。
- Rust 完整测试：通过。
- 正式 Tauri 构建：通过。
- 构建目标：Apple Silicon `aarch64-apple-darwin`。
- App 包版本：`3.2.1`。

正式 App 产物：

`src-tauri/target/aarch64-apple-darwin/release/bundle/macos/W4DJ RKB.app`

## Git 状态

代码定稿提交：`8d20c0e fix: anchor onboarding step five and release 3.2.1`

该提交已落在本地 `main`，未推送到远端。此说明文档作为交接材料新增，后续如需把文档也纳入版本提交，请单独提交该文件。
