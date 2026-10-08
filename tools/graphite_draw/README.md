# graphite_draw — Graphite 的 Python 命令皮

**一句话**：agent 用它画图。底下是 `graphite-cli` 四原语（save / inspect / exec / export），
上面是"建图层、加节点、连线、设参数"这样的人话操作。纯 Python 标准库，零依赖，python3 直接跑。

> 架构依据：`document/zh/CLI接口方案.md` §〇 形态 v2。Rust 侧 = API 引擎；本目录 = 命令皮 + 组合技。
> **状态：PY-2 联调完成**（2026-10-08）——全部形状经真进程 exec/inspect/export 实测锚定，
> serde 形状契约表见 `core/msgs.py` 模块头（联调校正只改那一处）。

## 两种用法

### 用法一：命令行（一条命令 = 一段组合技 = 一次冷启动）

```bash
GD=tools/graphite_draw/graphite_draw.py

# init：从零造编辑器形态空文档（legacy 模板 → save 原语转换；inspect 可验 source=editor）
python3 $GD init doc.gdd

# 四原语直通
python3 $GD save doc.graphite -o doc.gdd
python3 $GD inspect doc.gdd --pretty
python3 $GD exec doc.gdd --json-string '[…Message 数组…]' --save out.gdd
python3 $GD export doc.gdd -o out.png --width 512

# 原子命令八件套（= exec 原语组合，NodeId 由 Python 分配并回显）
python3 $GD create-layer doc.gdd --name bg --save out.gdd
python3 $GD add-node doc.gdd "PROTONODE:vector_nodes::generator_nodes::RectangleNode" \
    --position 100 200 --name rect --save out.gdd
python3 $GD connect doc.gdd 1 2 --to-input 1
python3 $GD disconnect doc.gdd 2 --to-input 1
python3 $GD set-param doc.gdd 2 --input 1 --value '{"F64": 200.0}'   # 变体须与现值一致
python3 $GD transform doc.gdd 1 --matrix 1 0 0 1 10 20   # DAffine2 列主序 6 元素
python3 $GD delete-node doc.gdd 2
python3 $GD select doc.gdd 1 2

# 公共选项（放子命令前）：--engine PATH  --resource-cache DIR  --timeout SEC
```

每条原子命令 stdout 回一行 JSON：`node_id_map`（锚点→NodeId）、`message_count`、`exec`
（`error_count`/`events`/`saved_path`）。出错走 stderr + 非零退出码。

**注意**：原子命令每次调用都是一次冷启动（约 1~3s）。批量操作请用用法二攒批。

### 用法二：库形态（agent skill 推荐：链式攒批，一次注入）

```python
import sys; sys.path.insert(0, "tools/graphite_draw")
from core.engine import Engine
from core.session import DrawSession
from core import msgs

engine = Engine()          # 自动探测 target/{debug,release}/graphite-cli；或 Engine(binary=…) / $GRAPHITE_CLI_BIN

engine.init("doc.gdd")     # 空文档起步

s = DrawSession("doc.gdd", engine)   # 一段组合技 = 一个 DrawSession
layer = s.create_layer(name="layer")
rect  = s.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode",
                   position=(0, 0), name="rect")
merge = s.add_node("NETWORK:Merge", name="merge")
s.connect(rect, merge)                # 参数可用返回的 NodeId 或锚点名
s.connect(merge, layer, to_input=1)   # → 图层内容输入
s.set_param("rect", 1, msgs.tagged_f64(200.0))

result = s.commit(save_to="out.gdd")  # ← 此时才真正调进程，一次冷启动执行全部
print(result.node_id_map)             # {"layer": 1, "rect": 2, "merge": 3}
print(result.ok)                      # exec error_count==0 即 True
```

要点：

- **攒批**：`s.xxx()` 只攒 Message，不碰进程；`commit()` 才注入（exec 的 `--file` 通道）。
- **锚点**：`name=` 起的名字在本段内随处可当 NodeId 用，`commit()` 回 `node_id_map`
  供下一段接驳（跨段用 inspect 拿到的数字 id）。
- **逃生门**：任何原生 Message 直接 `engine.exec_messages(doc, [msgs.node_graph(…)])`。
- **一层会话一次性**：commit 过的 DrawSession 别复用，新开一段。

## 事务语义（PY-2 实测，重要）

`graphite-cli exec` **整批自动包** `StartTransaction → 消息 → EndTransaction`——这是唯一事务层：

- ❌ Python 层**不要**再叠加 `AddTransaction`（实测叠加 = exec 报 0 错但重开值回滚：
  内层事务对提前提交，外层 EndTransaction 走 Finished 分支不落 registry 快照）；
- ❌ `CreateNodeFromContextMenu` 必须 `add_transaction=False`（True 同样静默丢节点）；
- ✅ 批次以 `RunDocumentGraph` 收尾（无害且触发重编译/渲染）；
- ✅ `DocumentMessage::DeleteNode` 内嵌事务，别用——用 `NodeGraphMessage::DeleteNodes`。

以上防回归均有测试锚（`tests/test_engine.py::test_exec_double_transaction_guard_documented`）。

## 实测可用的节点类型

`add-node` 的类型参数是 serialized 标识符（**proto 路径猜错会静默建点失败**——编辑器
对未知类型只 log 不报错，后续连线才炸）。已实测可用：

- `PROTONODE:vector_nodes::generator_nodes::RectangleNode`
- `NETWORK:Merge`

更多类型：用 `graphene-cli list-node-identifiers` 查（编译器栈），或对现成文档 `inspect`
读 `identifier` 字段。显示名（"Rectangle"）→ identifier 的解析器留待后续（需节点注册表）。

## export 前置：artboard

编辑器链导出（`SubmitDocumentExport`）**要求文档含 artboard**，否则弹 Error（实测）。
组合技里先 `msgs.new_artboard(id, dimensions=(w,h))` 再 export。看
`examples/end_to_end.py`。

## 模块地图

| 文件 | 职责 | 联调轮改它？ |
|---|---|---|
| `core/msgs.py` | **所有 serde JSON 形状**（含实测证据等级表、事务语义、init 模板、NewArtboard） | ✅ 形状校正只改这一处 |
| `core/engine.py` | subprocess 封装四原语 + init 组合技（二进制定位、超时、错误传播、stdout 解析） | 契约变动时 |
| `core/session.py` | DrawSession 组合技：攒批、锚点、RunGraph 收尾 | 逻辑问题才动 |
| `core/ids.py` | NodeId 整型递增分配器（0 保留给 ROOT_PARENT） | 不动 |
| `graphite_draw.py` | CLI 入口（argparse） | 加命令时 |
| `examples/end_to_end.py` | 端到端演示（init→组合技→inspect→export，全真实进程） | — |

## 测试与演示

```bash
# 单测（74 个：构造层 + 真进程联调，skip 已清零；联调测试需先 cargo build -p graphite-cli）
python3 -m unittest discover tools/graphite_draw/tests

# 端到端演示（init → 3节点2线1参组合技 → inspect 断言 → export SVG+PNG）
python3 tools/graphite_draw/examples/end_to_end.py
```

## 给后续联调轮的话

1. 形状校正只动 `core/msgs.py`（模块头有实测证据表），同步改 `tests/test_msgs.py`
   的内联期望值。
2. `exec` stdout 契约：`{events, error_count, saved_path?}`（exec.rs::ExecSummary）；
   inspect：`{source, nodeCount, nodes, wires, layers}`；export：
   `{saved_path, bytes, format, width, height, scale}`。Rust 侧若改字段，改 engine.py 的
   `*Result.from_stdout`。
3. 联调硬闸（CLI 方案 §五）：save → 重开 → 节点/参数一致 round-trip，每批必跑
   （`test_exec_set_param_value_persists` 即此闸的自动化形态）。
