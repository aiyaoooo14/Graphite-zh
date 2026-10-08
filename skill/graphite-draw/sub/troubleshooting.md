# troubleshooting：静默失败排查 + 已知坑

## 目的

两类故障的固定排查顺序：显式报错、零报错无效果。

## 命令（排查三板叉，按序执行）

```python
# 叉 1：exec 摘要
r = s.commit(save_to=doc)
r.exec_result.error_count   # >0 → 读事件
r.exec_result.errors        # [{"kind":"error","message_index":N,"detail":"…"}]

# 叉 2：重开比对（改动是否真的落盘）
before = engine.inspect(doc)
# …重跑操作…
after = engine.inspect(doc)
after.node_count, after.param_of(node_id, idx)   # 与预期比对

# 叉 3：类型核对（建点失败最常见的根因；终端执行）
#   "$GRAPHITE_CLI_BIN" node-list --filter <关键词>
# 比对 add_node 用的 identifier 是否逐字符一致
```

## 参数表（排查动作 → 检查对象）

| 叉 | 检查对象 | 字段 / 命令 | 判读 |
|---|---|---|---|
| 1 | exec 摘要 | `error_count`、`events[].kind=="error"` 的 `message_index`+`detail` | >0 → 定位到第 N 条消息 |
| 2 | 文档重开态 | `nodeCount`、`wires`、`param_of(id, idx)` | 操作前后无变化 = 未落盘（坑 2） |
| 3 | 节点类型 | `node-list --filter` 输出的 `identifier` | 与 add_node 参数逐字符比对 |

## 成功判据

排查完成的标志：叉 1 定位错误项 → 叉 2 确认落盘 → 叉 3 排除类型错。三者过后仍失败，
收集 `r.exec_result.events` 与两次 inspect 输出上报。

## 常见错误（已知坑清单）

| # | 现象 | 根因 | 处置 |
|---|---|---|---|
| 1 | exec 报 `unknown variant 'node_id'` / `unknown variant '…'` | 手拼 Message 缺变体名层（如 `{"NodeGraph":{"CreateWire":{…}}}` 写成 `{"NodeGraph":{…}}`） | 全部经 msgs 构造函数 |
| 2 | exec 0 错、重开无节点/值回滚 | 批次自带事务叠加 exec 自动事务 | 删 AddTransaction；add_transaction 保持 False（sub/session.md 红线） |
| 3 | add_node 无声失败、connect 报 `Node N not found in input_from_connector` | proto 路径猜错（例：Blur 实际在 raster_nodes::filter，不是 vector_nodes::filter_nodes） | node-list 查 identifier，逐字符核对 |
| 4 | export 报 dialog `Error` | 文档无 artboard | `msgs.new_artboard(...)` 前置 |
| 5 | set_param 后值没变 | TaggedValue 变体与现值不一致 | inspect `params` 看现值变体，同变体再设 |
| 6 | nodeCount 比自己建的节点多 | `transform(layer,…)` 使编辑器在层链自动插 TransformNode（随机大 NodeId） | 预期值按 +1/次 transform 计；见 recipes/parametric-repeat.md 判据 |
| 7 | exec/export 报 "compiler-stack .gdd" | 文档来自 graphene-cli（无内嵌编辑器文档） | init 新文档或换编辑器形态 .gdd |
| 8 | `graphite-cli exited 1` + stderr 有 serde 报错 | Message JSON 形状错误（数组/对象/字段名层级错） | 对照 cli/docs/message-shapes.md；用 msgs 构造函数 |
| 9 | `graphite-cli binary not found` | 未构建 / env 未设 | `cargo build -p graphite-cli`；或 `export GRAPHITE_CLI_BIN=<path>` |
| 10 | 在原进程反复重试无效 | exec 是一锤子进程，重试 = 新进程重开文档 | 先叉 2 比对落盘态再重试 |
