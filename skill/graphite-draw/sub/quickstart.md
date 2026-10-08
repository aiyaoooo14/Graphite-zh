# quickstart：从零画一张图

## 目的

四步出图：init 空文档 → 组合技建图 → 补 artboard → 导出 SVG/PNG。代码照抄可跑。

## 命令

前置：`cargo build -p graphite-cli`（或 `$GRAPHITE_CLI_BIN` 已指向现成二进制）。

```python
import os, sys
sys.path.insert(0, os.environ.get("GRAPHITE_DRAW_SKILL", "tools/graphite_draw"))
from core.engine import Engine
from core.session import DrawSession
from core import msgs

engine = Engine()

# 1. init 空文档
doc = "/tmp/my-artwork.gdd"
engine.init(doc)

# 2. 组合技：图层 + 矩形 + 合并，两线一参（一次 exec = 一次冷启动）
s = DrawSession(doc, engine)
layer = s.create_layer(name="layer")                                   # NodeId 1
rect  = s.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode",
                   position=(0, 0), name="rect")                       # NodeId 2
merge = s.add_node("NETWORK:Merge", name="merge")                      # NodeId 3
s.connect(rect, merge)                # rect:0 → merge:0
s.connect(merge, layer, to_input=1)   # merge:0 → layer 内容输入
s.set_param(rect, 1, msgs.tagged_f64(123.0))  # 矩形宽 123
r = s.commit(save_to=doc)

# 3. artboard（export 前置，单独一段）
engine.exec_messages(doc, [msgs.new_artboard(900, dimensions=(400.0, 300.0))], save_to=doc)

# 4. 导出
engine.export(doc, "/tmp/my-artwork.svg")
engine.export(doc, "/tmp/my-artwork.png", width=512)
```

## 参数表

| 位置 | 参数 | 值 |
|---|---|---|
| init | doc | .gdd 输出路径 |
| add_node | 第 1 参 | serialized 类型（`node-list` 查得，禁止猜） |
| add_node | position | 画布网格坐标 `(x, y)` |
| set_param | 第 2 参 | 输入索引；第 3 参 `msgs.tagged_f64(v)` |
| new_artboard | dimensions | 画板 `(宽, 高)`，决定导出默认尺寸 |
| export | width | PNG 目标宽（高按比例） |
| commit | save_to | 段末存盘路径 |

## 成功判据

1. `r.ok is True` 且 `r.node_id_map == {"layer": 1, "rect": 2, "merge": 3}`。
2. `/tmp/my-artwork.svg` 存在，根元素 `width="400"`。
3. `/tmp/my-artwork.png` 存在，前 8 字节 `\x89PNG\r\n\x1a\n`，尺寸 512x384。
4. `engine.inspect(doc)` → `source=="editor"`、`nodeCount==4`、`param_of(2,"1")=={"F64":123.0}`。

## 常见错误

| 现象 | 原因 | 处置 |
|---|---|---|
| `graphite-cli binary not found` | 未构建/未设 env | `cargo build -p graphite-cli` 或设 `$GRAPHITE_CLI_BIN` |
| exec 报 `unknown variant 'node_id'` | 手写 Message 漏变体名层 | 用 msgs 构造函数，不手拼 JSON |
| exec 报 0 错但重开无节点 | 批次带事务（见 sub/session.md 红线） | 去掉 AddTransaction / 改 add_transaction:false |
| export 报 dialog Error | 文档无 artboard | 执行第 3 步 |
