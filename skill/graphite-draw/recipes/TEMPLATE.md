# 配方模板（新配方的唯一起点）

用法：整文件复制为 `recipes/<配方名>.md` → 逐段填写 → 删除全部【填写要求】块 → 真跑全判据 → INDEX.md 登记。未走本模板的配方不入 INDEX。

## 元信息

【填写要求】四项全填。配方名 = 文件名（小写连字符）。验证状态只允许两种值：`真跑通过 YYYY-MM-DD`（本机全部判据跑过后填）或 `未验证`（未验证的配方不得登记 INDEX）。来源写一句任务出处（什么需求/会话沉淀的）。

- 配方名：<配方名>
- 适用场景：<一行；写"什么需求时用"，不写实现方式>
- 验证状态：真跑通过 YYYY-MM-DD
- 来源：<任务/会话出处>

## 目的与效果图

【填写要求】两行以内：产出什么 + 可 grep 的外观证据（如"SVG 含 N 个 linearGradient"）。不写实现原理、不写教程。

<产出物一句话描述。>

效果图判据：<可 grep 的字符串 / 计数 / 尺寸>

## 参数表

【填写要求】≥2 个参数，否则不构成配方（硬编码数字不合格）。参数必须全部出现在下方代码的参数区，一一对应。每行：参数名 / 类型 / 缺省值 / 改动效果（改它什么跟着变，含判据联动）。

| 参数 | 类型 | 缺省 | 改动效果 |
|---|---|---|---|
| <NAME> | <类型> | <值> | <什么变> |

## 代码

【填写要求】完整可复制（从 env 双形态到导出与判据自检，不省略、不写"省略类似"）。结构固定：env 双形态 → 参数区 → init → DrawSession 建图 → commit(save_to) → artboard → export → 判据自检段。节点类型全部经 node-list 查得（红线：禁止猜）；批次内禁止自带事务（红线：无 AddTransaction / add_transaction:true）；export 前必须有 artboard。

```python
import os, sys
sys.path.insert(0, os.environ.get("GRAPHITE_DRAW_SKILL", "tools/graphite_draw"))
from core.engine import Engine
from core.session import DrawSession
from core import msgs

# ── 参数区（与参数表一一对应）─────────────────
<NAME> = <缺省值>

# ── 建图 ─────────────────────────────────────
doc = "<OUT>/<配方名>.gdd"
engine = Engine()
engine.init(doc)

s = DrawSession(doc, engine)
<建图操作；锚点用 name=>
r = s.commit(save_to=doc)
assert r.ok, r.exec_result.errors

engine.exec_messages(doc, [msgs.new_artboard(<id>, dimensions=(<W>, <H>))], save_to=doc)
svg = engine.export(doc, "<OUT>/<配方名>.svg")
png = engine.export(doc, "<OUT>/<配方名>.png", width=<宽>)

# ── 判据自检（与"产物判据"段一一对应）─────────
import re
txt = open(svg.saved_path).read()
<assert 们>
print("OK:", svg.saved_path, png.saved_path)
```

## 产物判据

【填写要求】每条 = 可执行断言，与代码的判据自检段一一对应。**数值必须真跑后填，禁止推算**——先跑通再回来填数。node_count 判据给构成式（每项注明显式节点 / 自动插入的 TransformNode）；SVG 判据给 grep 计数式；message_count 给操作数构成式。

1. `r.ok is True`；`r.message_count == <操作数构成式>`。
2. SVG/PNG 尺寸：<实测值>。
3. SVG grep：<计数式>（真跑实测值）。
4. `st.node_count == <构成式>`（真跑实测值）。

## 已知坑

【填写要求】本配方实际触发的坑逐条列出；已有坑引用 sub/troubleshooting.md 的坑号（如"坑 6：TransformNode 自动插入"），新坑写全三栏。无坑保留表头并写"（无）"。

| # | 现象 | 根因 | 处置 |
|---|---|---|---|
| 1 | <现象> | <根因> | <处置> |

## 入库流程（填写完成后执行）

1. 删除本文件全部【填写要求】块与占位符。
2. 真跑配方代码，全部判据通过；回填"验证状态"的日期。
3. INDEX.md 登记一行（配方名 / 适用场景 / 文件名）。
4. 同步安装副本：`cp skill/graphite-draw/recipes/<配方名>.md ~/.agents/skills/graphite-draw/recipes/`（SKILL.md / INDEX.md 若有改动一并 cp）。
