---
name: graphite-draw
description: 用 Graphite 无头引擎画图。触发词：画图、绘图、矢量图、logo、图标、SVG、PNG、graphite、node graph。生成 .gdd 文档并导出 SVG/PNG。
---

# graphite-draw

能力：init 空文档 → 组合技建图（建点/连线/设参/变换）→ 导出 SVG/PNG → inspect 查结构。
执行体：`graphite-draw` Python 皮（Engine 四原语封装），二进制经 `$GRAPHITE_CLI_BIN`。

## 路由表

| 任务 | 读 |
|---|---|
| 从零画一张图（最小流程） | sub/quickstart.md |
| 建点 / 连线 / 设参 / 查节点类型 | sub/nodes.md |
| 导出 SVG / PNG | sub/export.md |
| 多操作攒一批（省冷启动）/ 事务红线 | sub/session.md |
| 查文档结构 / 存盘转换 | sub/document.md |
| 报错排查 / 没报错但没效果 | sub/troubleshooting.md |
| 完整案例（照抄改参数） | recipes/INDEX.md |
| 新配方填写模板 | recipes/TEMPLATE.md |
| 安装到本机 | install.md |

## 硬性规则

1. 节点 proto 路径必须 `node-list` 查得，禁止猜（sub/nodes.md）。
2. 批次内禁止自带事务（AddTransaction / add_transaction:true）——叠加 = 静默回滚（sub/session.md）。
3. export 前必须有 artboard（sub/export.md）。
4. 绘制任务收尾：方案可复用（非平凡节点组合 + ≥2 个可参数化点 + 判据可执行）时，向用户提议沉淀配方；同意后按 recipes/TEMPLATE.md 填写，存 recipes/<名称>.md 并在 INDEX.md 登记。
