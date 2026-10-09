# 配方：gradient-bloom（渐变花瓣花：径向重复 × 渐变填充 × 布尔圆环）

## 适用场景

径向对称图标/logo：N 个图形绕中心旋转平铺、渐变填充、布尔挖环。改花瓣数/形状半径/渐变色/布尔半径四个参数即得不同花型。

## 代码

```python
import os, sys, math
sys.path.insert(0, os.environ.get("GRAPHITE_DRAW_SKILL", "tools/graphite_draw"))
from core.engine import Engine
from core.session import DrawSession
from core import msgs

# ── 参数区 ──────────────────────────────────────────────
OUT = "/tmp/opencode/p5b"                       # 产物目录（须已存在）
DOC = f"{OUT}/gradient-bloom.gdd"
CANVAS = 512.0            # 画板边长
N = 8                     # 花瓣数（径向重复）
PETAL_RX = 34.0           # 花瓣椭圆短半径
PETAL_RY = 80.0           # 花瓣椭圆长半径
PETAL_DIST = 128.0        # 花瓣中心到花心距离
RING_R_OUT = 64.0         # 圆环外圆半径
RING_R_IN = 42.0          # 圆环内圆半径（布尔减掉）
DOT_R = 26.0              # 花蕊半径

def color(r, g, b, a=1.0):
    return {"alpha": a, "blue": b, "green": g, "linear": True, "red": r}

PETAL_RAMP = {"gradient_space": "OkLab",
              "stops": {"color": [color(0.86, 0.15, 0.45), color(1.00, 0.62, 0.10)]}}
DOT_RAMP = {"gradient_space": "OkLab",
            "stops": {"color": [color(1.00, 0.85, 0.25), color(0.90, 0.35, 0.05)]}}
RING_COLOR = color(0.45, 0.16, 0.28)

def petal_matrix(i):
    """第 i 瓣图层矩阵：平移花瓣到 (0,-DIST)，绕花心转 i*(360/N)，落到画布中心。"""
    th = 2.0 * math.pi * i / N
    c, sn = math.cos(th), math.sin(th)
    tx = CANVAS / 2 + PETAL_DIST * sn
    ty = CANVAS / 2 - PETAL_DIST * c
    return msgs.daffine2([c, sn, -sn, c, tx, ty])

# ── 建图 ────────────────────────────────────────────────
engine = Engine()
engine.init(DOC)

s = DrawSession(DOC, engine)
# N 个花瓣层：Ellipse → Fill(渐变) → layer(旋转矩阵)
for i in range(N):
    layer = s.create_layer(name=f"petal{i}")
    ell = s.add_node("PROTONODE:vector_nodes::generator_nodes::EllipseNode", name=f"petal_ell{i}")
    fill = s.add_node("PROTONODE:graphene_core::vector::FillNode", name=f"petal_fill{i}")
    s.connect(ell, fill, to_input=0)
    s.connect(fill, layer, to_input=1)
    s.set_param(ell, 1, msgs.tagged_f64(PETAL_RX))
    s.set_param(ell, 2, msgs.tagged_f64(PETAL_RY))
    s.set_param(fill, 1, msgs.tagged("Gradient", PETAL_RAMP))
    s.transform(layer, petal_matrix(i))

# 圆环层：两圆 merge(Base/Content) → Boolean(SubtractFront=外减内) → Fill(纯色) → layer
ring_layer = s.create_layer(name="ring")
cA = s.add_node("PROTONODE:vector_nodes::generator_nodes::CircleNode", name="ring_out")
cB = s.add_node("PROTONODE:vector_nodes::generator_nodes::CircleNode", name="ring_in")
merge = s.add_node("NETWORK:Merge", name="ring_merge")
boolean = s.add_node("PROTONODE:path_bool_nodes::BooleanOperationNode", name="ring_boolean")
ring_fill = s.add_node("PROTONODE:graphene_core::vector::FillNode", name="ring_fill")
s.connect(cA, merge, to_input=0)          # Base = 先连（被减方）
s.connect(cB, merge, to_input=1)          # Content = 后连（减数）
s.connect(merge, boolean, to_input=0)
s.connect(boolean, ring_fill, to_input=0)
s.connect(ring_fill, ring_layer, to_input=1)
s.set_param(cA, 1, msgs.tagged_f64(RING_R_OUT))
s.set_param(cB, 1, msgs.tagged_f64(RING_R_IN))
s.set_param(boolean, 1, msgs.tagged("BooleanOperation", "SubtractFront"))
s.set_param(ring_fill, 1, msgs.tagged("Color", RING_COLOR))
s.transform(ring_layer, msgs.translation_matrix(CANVAS / 2, CANVAS / 2))

# 花蕊层：Circle → Fill(渐变) → layer
dot_layer = s.create_layer(name="dot")
dot = s.add_node("PROTONODE:vector_nodes::generator_nodes::CircleNode", name="dot_circle")
dot_fill = s.add_node("PROTONODE:graphene_core::vector::FillNode", name="dot_fill")
s.connect(dot, dot_fill, to_input=0)
s.connect(dot_fill, dot_layer, to_input=1)
s.set_param(dot, 1, msgs.tagged_f64(DOT_R))
s.set_param(dot_fill, 1, msgs.tagged("Gradient", DOT_RAMP))
s.transform(dot_layer, msgs.translation_matrix(CANVAS / 2, CANVAS / 2))

r = s.commit(save_to=DOC)
assert r.ok, r.exec_result.errors

engine.exec_messages(DOC, [msgs.new_artboard(900, dimensions=(CANVAS, CANVAS))], save_to=DOC)
svg = engine.export(DOC, f"{OUT}/gradient-bloom.svg")
png = engine.export(DOC, f"{OUT}/gradient-bloom.png", width=512)

# ── 判据自检 ────────────────────────────────────────────
import re
txt = open(svg.saved_path).read()
assert len(re.findall(r'<linearGradient\b', txt)) == N + 1        # 花瓣渐变 + 花蕊渐变
assert len(re.findall(r'fill="url\(#', txt)) == N + 1
assert txt.count("<path") == N + 2                                 # N 瓣 + 圆环 + 花蕊
assert len(re.findall(r'<path d="[^"]*M[^"]*M', txt)) == 1         # 双 M 子路径 = 布尔圆环
st = engine.inspect(DOC)
assert st.node_count == 4 * N + 12, st.node_count                  # 见判据 5 的构成式
print("OK:", svg.saved_path, png.saved_path)
```

## 参数表

| 参数 | 作用 | 改动效果 |
|---|---|---|
| N | 花瓣数 | 判据随 N 变：linearGradient=N+1、path=N+2、node_count=4N+12 |
| PETAL_RX / PETAL_RY | 花瓣椭圆两轴 | 花瓣形状 |
| PETAL_DIST | 花瓣离花心距离 | 花型开合 |
| PETAL_RAMP / DOT_RAMP | 渐变 stop 色 | `color(r,g,b)` 改数值；stops 数组可加中间色 |
| RING_R_OUT / RING_R_IN | 布尔外/内圆半径 | 环宽；内>外则环消失 |
| `"BooleanOperation"` op | 布尔操作 | `Union`/`SubtractFront`/`SubtractBack`/`Intersect`/`Difference` |
| CANVAS | 画板边长 | petal_matrix 与两处 translation 同源，联动改 |
| 形状类型 | Ellipse/Circle 换 node-list 查得的 generator | 花瓣/花芯形态 |

## 预期产物判据（N=8 默认参数实测值）

1. `r.ok is True`，`r.message_count == 97`（花瓣 9×8 + 圆环 16 + 花蕊 8 + RunGraph）。
2. SVG/PNG 均 512x512。
3. SVG 结构：`<linearGradient>` 计数 = **9**（8 瓣 + 花蕊）、`fill="url(#"` 引用 = **9**、
   `radialGradient` = 0、`<path>` 元素 = **10**（8 瓣 + 圆环 + 花蕊）。
4. SubtractFront 证据：恰好 1 个 `<path>` 含两个 `M` 子路径（外圆−内圆的环）。
5. inspect：`st.node_count == 4N + 12`（N=8 → 44，实测）。构成式：
   每瓣 4（layer+Ellipse+Fill+自动 Transform）×N ＋ 圆环 7（6 显式+1 Transform）＋
   花蕊 4（3 显式+1 Transform）＋ artboard 1。

## 已知坑

| 坑 | 处置 |
|---|---|
| BooleanOperation 枚举值查不到全集（node-list default 只列 `Union`） | 合法值：`Union`/`SubtractFront`/`SubtractBack`/`Intersect`/`Difference`（本配方已实测 SubtractFront）；新值先 exec 试错读报错 |
| 同一 input 第二次 connect 顶掉首线 | 圆环两圆必须分接 Merge 的 Base(0)/Content(1) 双口（代码已按此写） |
| `transform(layer,…)` 自动插 TransformNode | node_count 判据已含；勿对自动插入节点建锚点 |
| FillNode.Paint 跨变体（默认 Color 设成 Gradient） | `msgs.tagged("Gradient", ramp)` 一次设入即可（实测支持）；ramp 形状 = `{"gradient_space":"OkLab","stops":{"color":[…]}}` |
