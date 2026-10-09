# 配方：参数化重复结构（N 格网格）

## 适用场景

同一图形按 N 格网格平铺（贴纸阵、图标网格、重复纹理底）。改 `N`、格距、格宽三个参数即得不同布局。

## 代码

```python
import os, sys
sys.path.insert(0, os.environ.get("GRAPHITE_DRAW_SKILL", "tools/graphite_draw"))
from core.engine import Engine
from core.session import DrawSession
from core import msgs

N = 3          # 每行格数
PITCH = 90     # 格距（px）
SIZE = 60.0    # 方块边长
doc = "/tmp/parametric-grid.gdd"

engine = Engine()
engine.init(doc)

s = DrawSession(doc, engine)
artboard_id = 10000 + N   # 避开 DrawSession 自增段（1..2N）
for i in range(N):
    x = i * PITCH
    layer = s.create_layer(name=f"cell{i}")
    rect = s.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode",
                      position=(x, 0), name=f"rect{i}")
    s.connect(rect, layer, to_input=1)                       # 图形 → 图层内容
    s.set_param(rect, 1, msgs.tagged_f64(SIZE))              # 宽
    s.set_param(rect, 2, msgs.tagged_f64(SIZE))              # 高
    s.transform(layer, msgs.translation_matrix(x + 20, 50))  # 格位平移
r = s.commit(save_to=doc)
assert r.ok, r.exec_result.errors

engine.exec_messages(
    doc,
    [msgs.new_artboard(artboard_id, dimensions=(float(N * PITCH), 200.0))],
    save_to=doc,
)

svg = engine.export(doc, "/tmp/parametric-grid.svg")
png = engine.export(doc, "/tmp/parametric-grid.png", width=600)
print(svg.saved_path, svg.width, svg.height)
print(png.saved_path, png.width, png.height)
```

## 参数说明

| 参数 | 作用 | 改动效果 |
|---|---|---|
| N | 格数 | 节点数/导出宽度随之变（画板宽 = N×PITCH） |
| PITCH | 格距 | 格间距 |
| SIZE | 方块边长 | set_param 值 |
| translation_matrix(x+20, 50) | 每格的层平移 | 网格布局公式 |
| artboard dimensions | 画板 | 需覆盖全部格位 |

## 预期产物判据

1. `r.ok is True`；`r.message_count == N*6 + 1`（每格 6 操作 + RunGraph）。
2. `r.node_id_map` 含 `cell0..cell{N-1}`、`rect0..rect{N-1}`，共 2N 个锚点。
3. inspect：`st.node_count == 3N + 1`——N 层 + N 矩形 + artboard + **N 个编辑器自动插入的
   TransformNode**（`transform(layer,…)` 在层链插入 `transform_nodes::transform_nodes::TransformNode`，
   随机大 NodeId，属编辑器行为，不计入锚点）。
4. SVG：宽 = N×PITCH（N=3 → 270x200）；PNG 宽 = 600。
