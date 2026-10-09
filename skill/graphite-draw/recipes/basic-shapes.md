# 配方：矩形 + 合并的基础图形

## 适用场景

单个几何图形挂到图层并导出；改参数可换任意 node-list 查得的图形类型。

## 代码

```python
import os, sys
sys.path.insert(0, os.environ.get("GRAPHITE_DRAW_SKILL", "tools/graphite_draw"))
from core.engine import Engine
from core.session import DrawSession
from core import msgs

doc = "/tmp/basic-shapes.gdd"
engine = Engine()
engine.init(doc)

s = DrawSession(doc, engine)
layer = s.create_layer(name="layer")
rect  = s.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode",
                   position=(0, 0), name="rect")
merge = s.add_node("NETWORK:Merge", name="merge")
s.connect(rect, merge)                        # 图形 → 合并
s.connect(merge, layer, to_input=1)           # 合并 → 图层内容
s.set_param(rect, 1, msgs.tagged_f64(123.0))  # 宽 123
r = s.commit(save_to=doc)
assert r.ok, r.exec_result.errors

engine.exec_messages(doc, [msgs.new_artboard(900, dimensions=(400.0, 300.0))], save_to=doc)

svg = engine.export(doc, "/tmp/basic-shapes.svg")
png = engine.export(doc, "/tmp/basic-shapes.png", width=512)
print(svg.saved_path, svg.width, svg.height)
print(png.saved_path, png.width, png.height)
```

## 参数说明

| 参数 | 改法 |
|---|---|
| 图形类型 | add_node 第 1 参换 node-list 查得的 identifier |
| 尺寸 | set_param 的 f64 值（Rectangle input1=宽，input2=高） |
| 画板 | new_artboard 的 dimensions=(宽, 高) |
| 导出尺寸 | export 的 width 或 height（另一维按比例） |
| 矩形圆角 | `s.set_param(rect, 3, msgs.tagged_box_corners([r1, r2, r3, r4]))` |

## 预期产物判据

1. `r.node_id_map == {"layer": 1, "rect": 2, "merge": 3}`。
2. inspect：`st.node_count==4`（layer+rect+merge+artboard）、`st.param_of(2,"1")=={"F64":123.0}`。
3. SVG：400x300；PNG：512x384。
