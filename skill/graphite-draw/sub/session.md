# session：DrawSession 组合技 / 事务红线

## 目的

多操作攒一批，一次 commit 一次冷启动。以及事务的两条红线。

## 命令

```python
s = DrawSession(doc, engine)          # 一段组合技一个实例，commit 后废弃
a = s.create_layer(name="layer")      # 攒批：不触进程
b = s.add_node("<identifier>", position=(0, 0), name="b")
s.connect(a, b)
s.set_param(b, 0, msgs.tagged_f64(1.0))
s.transform(a, msgs.translation_matrix(50, 25))
r = s.commit(save_to=doc)             # ← 唯一触进程点；save_to=段末存盘
r.node_id_map                         # {"layer": 1, "b": 2, ...}
r.ok                                  # exec error_count==0
r.exec_result.events                  # 事件列表（错误项 kind=="error"）
```

锚点引用：本段内 `connect/set_param/transform/delete_node/select` 的节点参数可传
commit 返回的 NodeId（int）或 `name=` 锚点名（str）。跨段引用用 inspect 读回的数字 id。

## 事务红线（PY-2 实测，违反 = 静默回滚）

1. **批次内禁止 AddTransaction**。exec 整批已自动包 `StartTransaction→消息→EndTransaction`；
   叠加 AddTransaction 后 exec 报 0 错但改动不落盘（实测：参数值重开回滚）。
2. **禁止内嵌事务消息**：`CreateNodeFromContextMenu` 必须 `add_transaction:false`
   （msgs 构造函数已默认 False，勿改）；`DocumentMessage::DeleteNode` 内嵌事务，删除用
   `msgs.delete_nodes([...])`。

合规形态由 `s.pending_messages()` 产出：`[操作们…, RunDocumentGraph]`。

## 参数表

| API | 参数 | 值 |
|---|---|---|
| DrawSession | document / engine | 文档路径 / Engine 实例；document=None 为纯攒批（commit 不触进程） |
| commit | save_to | 段末存盘 .gdd 路径，可省 |
| transform | matrix | `msgs.daffine2([...6 元素...])` / `msgs.translation_matrix(x,y)` / `msgs.identity_matrix()` |

## 成功判据

1. `r.ok is True`；`r.message_count == 操作数 + 1`（RunGraph 收尾）。
2. `r.node_id_map` 含全部锚点名。
3. `json.dumps(r.messages)` 不含 `AddTransaction`（红线自检）。

## 常见错误

| 现象 | 原因 | 处置 |
|---|---|---|
| exec 0 错但重开无改动 | 红线 1 或 2 | 去掉自带事务；用 pending_messages() 自检 |
| `nothing to commit` | 空段 | 至少一个操作再 commit |
| `commit() needs an Engine` | document 给了但 engine=None | `DrawSession(doc, engine)` |
| `name '…' already bound` | 锚点名段内重复 | 换名 |
| 复用已 commit 的 session | 实例一次性 | 新建 DrawSession 开新段 |
