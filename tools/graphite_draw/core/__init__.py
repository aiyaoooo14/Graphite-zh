"""graphite_draw — Graphite 编辑器的 Python 命令皮（CLI 方案 §〇 形态 v2，PY-1 切片）。

包结构：
- ids.py      NodeId 分配器（组合技内整型递增，调用方自主分配）
- msgs.py     Message JSON 构造层（serde 形状常量集中处，联调轮只校正这一处）
- engine.py   Engine 类：graphite-cli 四原语的 subprocess 封装
- session.py  DrawSession：链式攒批组合技，一次 exec 注入 = 一次冷启动
"""
