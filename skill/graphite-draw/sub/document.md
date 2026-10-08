# document：init / inspect / save

## 目的

文档生命周期三操作：从零创建、读结构、格式转换。

## 命令

```python
engine.init("/tmp/doc.gdd")                      # 从零造编辑器形态空 .gdd
st = engine.inspect("/tmp/doc.gdd")              # 读结构（InspectionResult）
st.raw                                           # 原始 JSON dict
result = engine.save("/tmp/legacy.graphite", output="/tmp/doc.gdd")  # legacy → .gdd
```

CLI 形态：

```bash
python3 tools/graphite_draw/graphite_draw.py init /tmp/doc.gdd
python3 tools/graphite_draw/graphite_draw.py inspect /tmp/doc.gdd --pretty
python3 tools/graphite_draw/graphite_draw.py save /tmp/in.graphite -o /tmp/out.gdd
```

## 参数表

| API | 参数 | 值 |
|---|---|---|
| init | output | .gdd 路径；父目录须存在 |
| inspect | document | .gdd 或 .graphite 路径 |
| save | document / output | 输入路径 / 输出 .gdd 路径 |

## 成功判据

1. init：`engine.inspect(doc).source == "editor"` 且 `node_count == 0`。
2. inspect 返回字段：`source`（editor/registry）、`nodeCount`、`nodes[{id,type,identifier,display_name,is_layer,position,params}]`、`wires[{from:{node,output},to:{node,input}}]`、`layers[{id,name,children}]`。
3. save：`result.path` 是输出路径，`result.bytes_written > 0`；重开 inspect `source=="editor"`。

## 常见错误

| 现象 | 原因 | 处置 |
|---|---|---|
| `exec`/`export` 报 "compiler-stack .gdd" | 文档无内嵌编辑器文档（graphene-cli 产物） | 用 `inspect`（registry 路径）读；exec/export 换编辑器形态文档 |
| inspect `source=="registry"` | 同上 | 该文档不能 exec/export；重新 init |
| save 后 inspect 打不开 | 手改过 .gdd 内部文件 | .gdd 是容器，只经原语读写 |
