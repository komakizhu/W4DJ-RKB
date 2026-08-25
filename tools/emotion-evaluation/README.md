# W4DJ 情绪模型主观验收工具

这是工作区中的离线 HTML 工具，不属于 Dashboard。它不运行模型，只读取 W4DJ 导出的 `emotion-evaluation-manifest.json`，并收集四套系统的匿名主观比较结果。

## 生成 manifest

```bash
cargo run --manifest-path src-tauri/Cargo.toml \
  --bin export_emotion_evaluation_manifest -- \
  --database "/path/to/w4dj.sqlite3" \
  --output "/tmp/emotion-evaluation-manifest.json" \
  --count 100 --seed 20260822
```

缺少 `--database` 时也可以设置 `W4DJ_LIBRARY_PATH`。`--count 0` 表示导出全部可用歌曲。

## 打开页面

浏览器的目录权限要求通过本地 HTTP 服务打开页面，不要直接双击 HTML：

```bash
python3 -m http.server 1431 --directory tools/emotion-evaluation
```

打开 <http://127.0.0.1:1431/>，选择 manifest 和输出音频文件夹。页面每首歌先收集主观情绪，再显示匿名 A/B/C/D 卡片。完成后手动导出 JSON 或 CSV。

评测过程中可暂停、返回上一首或在汇总页修改上一首；音频缺失/无法播放和模型缺失不会进入胜率分母。

结果只保存在浏览器 IndexedDB，导出文件由用户明确点击生成，不修改 W4DJ 歌曲库或分析缓存。
