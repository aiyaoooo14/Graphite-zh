# export：SVG / PNG 导出

## 目的

经编辑器渲染链导出产物。前置条件：文档含 artboard。

## 命令

```python
# 前置：文档无 artboard 时先注入（id 任意未占用的 NodeId）
engine.exec_messages(doc, [msgs.new_artboard(900, dimensions=(400.0, 300.0))], save_to=doc)

svg = engine.export(doc, "/tmp/out.svg")                    # 默认尺寸 = artwork bounds
png = engine.export(doc, "/tmp/out.png", width=512)         # 高按比例
png = engine.export(doc, "/tmp/out.png", height=1024)       # 宽按比例
png = engine.export(doc, "/tmp/out.png", scale=2.0)         # 倍率
```

CLI 形态：

```bash
python3 tools/graphite_draw/graphite_draw.py export doc.gdd -o out.svg
python3 tools/graphite_draw/graphite_draw.py export doc.gdd -o out.png --width 512
```

## 参数表

| 参数 | 值 |
|---|---|
| -o 输出路径 | 扩展名决定格式：`.svg` / `.png`；其他扩展名被拒 |
| width | 目标像素宽（优先于 scale，内部两步导出校正） |
| height | 目标像素高（同上） |
| scale | 倍率，默认 1.0 |
| transparent | 接受；PNG 恒 RGBA 透明底 |

返回字段（ExportResult）：`saved_path`、`bytes`、`format`、`width`、`height`、`scale`。

## 成功判据

1. `svg.format == "svg"`，文件存在且含 `<svg`，`width/height` 与 artboard 比例一致。
2. PNG 前 8 字节 = `\x89PNG\r\n\x1a\n`；width==512 时高按 artboard 比例（400:300 → 512x384）。
3. 文档无 artboard 时 export **必失败**（editor dialog Error）——先跑前置。

## 常见错误

| 现象 | 原因 | 处置 |
|---|---|---|
| `Editor raised dialogs during export: ["Error"]` | 文档无 artboard | `msgs.new_artboard(...)` 后重试 |
| `Unsupported export extension .jpg` | 编辑器链只支持 svg/png | 换扩展名 |
| PNG 尺寸与 --width 不符 | width 单独给时高按比例 | 读返回值 width/height 确认 |
| SVG 空内容 | 文档无可见图层 | 图层须有内容节点（见 sub/quickstart.md） |
