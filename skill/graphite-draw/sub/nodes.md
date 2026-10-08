# nodes：建点 / 连线 / 设参 / 查类型

## 目的

在 DrawSession 里操作节点，以及查节点类型的唯一合法途径。

## 命令

查类型（**建点前必查；proto 路径禁止猜——猜错建点静默失败，这是红线**）：

```bash
"$GRAPHITE_CLI_BIN" node-list --filter rectangle | python3 -m json.tool
"$GRAPHITE_CLI_BIN" node-list | python3 -c "import json,sys; [print(n['identifier']) for n in json.load(sys.stdin)]"
```

输出：JSON 数组，字段 `identifier`（喂给 add_node 的 serialized 类型）、`display_name`、`category`、`inputs`、`outputs`。

操作（DrawSession 链式）：

```python
s = DrawSession(doc, engine)
node_id = s.add_node("<identifier 查得>", position=(x, y), name="锚点名")
s.connect(from_node, to_node, from_output=0, to_input=0)
s.disconnect(to_node, to_input=0)
s.set_param(node, input_index, msgs.tagged_f64(200.0))
s.delete_node(node)
s.select([node_a, node_b])
r = s.commit(save_to=doc)
```

## 参数表

| API | 参数 | 值 |
|---|---|---|
| add_node | node_type | node-list 的 `identifier` 原样（`PROTONODE:…` / `NETWORK:…` 前缀形态） |
| add_node | position | 可省；网格坐标 `(int, int)` |
| add_node | name | 锚点名；commit 后 `r.node_id_map[name]` 得 NodeId |
| connect | from_output / to_input | 输出/输入索引，从 node-list 的 `outputs`/`inputs` 数组下标取 |
| set_param | input_index | 节点 `inputs` 下标；值变体必须与 inspect 读回的现值变体一致 |
| set_param | 值构造 | `tagged_f64` / `tagged_bool` / `tagged_string` / `tagged_box_corners` |
| delete_node | node | NodeId 或锚点名 |
| select | nodes | NodeId / 锚点名混合列表 |

## 成功判据

1. `r.ok is True`（exec `error_count==0`）。
2. 重开 `engine.inspect(doc)`：`nodeCount` 增加；`wires` 含 `{"from":{"node":A,"output":X},"to":{"node":B,"input":Y}}`。
3. `structure.param_of(node_id, idx)` == 设入的值。

## 常见错误

| 现象 | 原因 | 处置 |
|---|---|---|
| add_node 无声失败，后续 connect 报 `Node N not found` | node_type 是猜的，编辑器对未知类型只 log | node-list 重新查 identifier |
| exec 报 `unknown variant '…'` | 手拼 Message 缺变体名层 | 全部经 msgs 构造函数 |
| set_param 后值没变 | 变体不匹配 | inspect 看现值变体，同变体再设 |
| set_param 的 input 索引对不上 | inputs 下标从 node-list 取，非 UI 显示序 | 以 node-list `inputs` 数组序为准 |
