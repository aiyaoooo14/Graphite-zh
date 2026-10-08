#!/usr/bin/env python3
"""端到端最终演示（PY-2 验收）：全真实进程，无人工干预。

流程：
  1. init 空文档（legacy 模板 → graphite-cli save → 编辑器形态 .gdd）
  2. DrawSession 组合技：建 3 节点（图层 + Rectangle + Blur）连 2 线设 1 参
     → commit（exec --save，一次冷启动）
  3. 另一段组合技：NewArtboard（export 的实测前置）→ commit --save
  4. inspect 断言：节点数/连线数/参数值
  5. export SVG + PNG，断言产物落盘且尺寸正确

前置：cargo build -p graphite-cli（二进制由 Engine 自动探测）。
跑法：python3 tools/graphite_draw/examples/end_to_end.py
"""

import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from core.engine import Engine, EngineError  # noqa: E402
from core.session import DrawSession  # noqa: E402
from core import msgs  # noqa: E402

# 节点类型用 PY-2 实测可用的 serialized 标识符（proto 路径猜错会静默建点失败，
# 这是编辑器行为——CreateNodeFromContextMenu 对未知类型只 log 不报错）。
RECTANGLE = "PROTONODE:vector_nodes::generator_nodes::RectangleNode"
MERGE = "NETWORK:Merge"

_step = [0]


def step(title):
	_step[0] += 1
	print("\n[%d] %s" % (_step[0], title))


def check(name, condition, detail=""):
	status = "PASS" if condition else "FAIL"
	print("    %s  %s%s" % (status, name, (" — " + detail) if detail else ""))
	if not condition:
		raise AssertionError(name + (" " + detail if detail else ""))


def main() -> int:
	workdir = tempfile.mkdtemp(prefix="graphite_draw_e2e_")
	print("workdir:", workdir)
	engine = Engine()
	print("engine:", engine.locate_binary())

	step("init 空文档（legacy 模板 → save 原语转换）")
	doc = os.path.join(workdir, "artwork.gdd")
	written = engine.init(doc)
	check("file written", os.path.isfile(written), written)
	structure = engine.inspect(doc)
	check("source=editor", structure.source == "editor", structure.source)
	check("nodeCount=0", structure.node_count == 0, str(structure.node_count))

	step("组合技：建 3 节点连 2 线设 1 参（一次 exec 注入 = 一次冷启动）")
	session = DrawSession(doc, engine)
	layer = session.create_layer(name="layer")
	rect = session.add_node(RECTANGLE, position=(0, 0), name="rect")
	merge = session.add_node(MERGE, position=(200, 0), name="merge")
	session.connect(rect, merge)                  # Rectangle → Merge:0
	session.connect(merge, layer, to_input=1)     # Merge → 图层内容输入
	session.set_param(rect, 1, msgs.tagged_f64(123.0))  # Rectangle 的 F64 参数
	result = session.commit(save_to=doc)
	check("exec ok", result.ok, "error_count=%d" % (result.exec_result.error_count if result.exec_result else -1))
	check("message_count=7 (6 操作 + RunGraph)", result.message_count == 7, str(result.message_count))
	check("node_id_map", result.node_id_map == {"layer": 1, "rect": 2, "merge": 3}, json.dumps(result.node_id_map))

	step("第二段组合技：NewArtboard（export 实测前置）")
	session2 = DrawSession(doc, engine)
	session2._operations.append(msgs.new_artboard(900, dimensions=(400.0, 300.0)))
	result2 = session2.commit(save_to=doc)
	check("exec ok", result2.ok)

	step("inspect 断言（重开，跨进程）")
	structure = engine.inspect(doc)
	# 4 节点 = 图层 + Rectangle + Merge + Artboard；wires 含编辑器自动接的 artboard 内容线。
	check("nodeCount=4", structure.node_count == 4, str(structure.node_count))
	check("wire rect→merge", {"from": {"node": 2, "output": 0}, "to": {"node": 3, "input": 0}} in structure.wires,
	      json.dumps(structure.wires))
	check("wire merge→layer:1", {"from": {"node": 3, "output": 0}, "to": {"node": 1, "input": 1}} in structure.wires)
	check("types", {n["type"] for n in structure.nodes} >= {"RectangleNode"},
	      json.dumps(sorted({str(n["type"]) for n in structure.nodes})))
	check("layer tree", bool(structure.layers) and structure.layers[0]["id"] == 900, json.dumps(structure.layers)[:120])
	check("param persists (rect:1 = 123.0)", structure.param_of(2, "1") == {"F64": 123.0},
	      json.dumps(structure.param_of(2, "1")))

	step("export SVG + PNG")
	svg_path = os.path.join(workdir, "artwork.svg")
	png_path = os.path.join(workdir, "artwork.png")
	svg = engine.export(doc, svg_path)
	check("svg saved", svg.saved_path == svg_path and os.path.isfile(svg_path), "bytes=%s" % svg.bytes)
	check("svg 400x300", (svg.width, svg.height) == (400, 300), "%sx%s" % (svg.width, svg.height))
	png = engine.export(doc, png_path, width=512)
	check("png saved", os.path.isfile(png_path), "bytes=%s" % png.bytes)
	check("png width=512", png.width == 512, "%sx%s" % (png.width, png.height))

	print("\n=== END-TO-END OK ===")
	print("artifacts:")
	for path in (doc, svg_path, png_path):
		print("  %s (%d bytes)" % (path, os.path.getsize(path)))
	return 0


if __name__ == "__main__":
	try:
		sys.exit(main())
	except (EngineError, AssertionError) as error:
		print("\nE2E FAILED: %s" % error, file=sys.stderr)
		sys.exit(1)
