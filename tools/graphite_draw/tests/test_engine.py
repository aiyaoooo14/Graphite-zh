import sys
from pathlib import Path

# unittest discover 从任意 cwd 跑都能导入 core/*（本文件相对 tools/graphite_draw 上溯一级）。
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

"""Engine 层测试。

划分：
- **不依赖 Rust 进程的构造/解析契约**（stdout 解析、二进制定位失败路径）→ 正常跑；
- **真调 graphite-cli 的联调**（PY-2 起，skip 清零，subprocess 真跑）：
  需要 `cargo build -p graphite-cli` 的产物（target/{debug,release}/graphite-cli 或
  $GRAPHITE_CLI_BIN）。测试文档用 engine.init() 现做（init 组合技本身也在被测）。

联调断言依据（PY-2 实验结论，见 core/msgs.py 模块头）：
- exec 去封装批次（裸操作 + RunGraph）值持久；
- CreateNodeFromContextMenu 必须 add_transaction=False（内嵌事务 = 静默丢失）；
- export 需要文档含 artboard。
"""

import json
import os
import tempfile
import unittest
from unittest import mock

from core.engine import Engine, EngineError, ExecResult, ExportResult, InspectResult, SaveResult
from core import msgs


class TestSaveStdoutParsing(unittest.TestCase):
	"""save 的 stdout 是文本行（main.rs 现状），宽松解析不炸。"""

	def test_parse_standard_line(self):
		result = Engine._parse_save_stdout("Saved a.graphite -> out/out.gdd (31748 bytes)\n")
		self.assertEqual(result.document, "a.graphite")
		self.assertEqual(result.path, "out/out.gdd")
		self.assertEqual(result.bytes_written, 31748)

	def test_parse_unexpected_output_keeps_raw(self):
		result = Engine._parse_save_stdout("something unexpected")
		self.assertIsInstance(result, SaveResult)
		self.assertEqual(result.raw, "something unexpected")
		self.assertIsNone(result.path)


class TestExecResultParsing(unittest.TestCase):
	"""exec 摘要 JSON 解析（PY-2 契约：events/error_count/saved_path）。"""

	def test_from_stdout_current_shape(self):
		line = json.dumps({
			"events": [{"kind": "save_document", "name": "x", "path": None, "bytes": 561}],
			"error_count": 0,
			"saved_path": "out.gdd",
		})
		result = ExecResult.from_stdout(line)
		self.assertEqual(result.error_count, 0)
		self.assertEqual(result.saved_path, "out.gdd")
		self.assertEqual(result.events[0]["kind"], "save_document")
		self.assertTrue(result.ok)

	def test_error_events_surface(self):
		line = json.dumps({
			"events": [{"kind": "error", "message_index": 1, "detail": "boom"}],
			"error_count": 1,
		})
		result = ExecResult.from_stdout(line)
		self.assertEqual(result.error_count, 1)
		self.assertEqual(len(result.errors), 1)
		self.assertEqual(result.errors[0]["message_index"], 1)
		self.assertFalse(result.ok)

	def test_saved_path_omitted_when_no_save(self):
		result = ExecResult.from_stdout(json.dumps({"events": [], "error_count": 0}))
		self.assertIsNone(result.saved_path)

	def test_from_non_json_raises(self):
		with self.assertRaises(EngineError):
			ExecResult.from_stdout("not json")


class TestInspectExportParsing(unittest.TestCase):
	def test_inspect_result_contract(self):
		raw = {
			"source": "editor", "nodeCount": 1,
			"nodes": [{"id": 5, "type": "RectangleNode", "params": {"1": {"F64": 100.0}}}],
			"wires": [], "layers": [],
		}
		result = InspectResult.from_stdout(json.dumps(raw))
		self.assertEqual(result.source, "editor")
		self.assertEqual(result.node_count, 1)
		self.assertEqual(result.param_of(5, "1"), {"F64": 100.0})
		self.assertIsNone(result.param_of(5, "9"))

	def test_export_result_contract(self):
		raw = {"saved_path": "e.png", "bytes": 123, "format": "png", "width": 512, "height": 384, "scale": 1.28}
		result = ExportResult.from_stdout(json.dumps(raw))
		self.assertEqual((result.saved_path, result.format, result.width, result.scale),
		                 ("e.png", "png", 512, 1.28))


class TestBinaryLocation(unittest.TestCase):
	"""定位失败路径（不启动进程）。"""

	def test_explicit_missing_binary_raises_without_fallback(self):
		engine = Engine(binary="/nonexistent/path/graphite-cli")
		with self.assertRaises(EngineError):
			engine.locate_binary()

	def test_explicit_env_missing_raises(self):
		engine = Engine()
		with mock.patch.dict(os.environ, {"GRAPHITE_CLI_BIN": "/nonexistent/env/graphite-cli"}):
			with self.assertRaises(EngineError):
				engine.locate_binary()


class TestIntegration(unittest.TestCase):
	"""真调 graphite-cli 的原语联调（PY-2：skip 清零）。

	前置：cargo build -p graphite-cli 的产物可被 Engine.locate_binary() 找到。
	"""

	@classmethod
	def setUpClass(cls):
		cls.engine = Engine()
		cls.binary = cls.engine.locate_binary()  # 二进制缺失 → 明确报错（联调环境要求）
		cls.tmp = tempfile.TemporaryDirectory(prefix="graphite_draw_py2_")
		cls.doc = os.path.join(cls.tmp.name, "it.gdd")
		cls.engine.init(cls.doc)  # 空文档（同时就是 init 组合技的联调）

	@classmethod
	def tearDownClass(cls):
		cls.tmp.cleanup()

	# ── init 验收 ──

	def test_init_document_inspects_as_editor_empty(self):
		"""任务书验收：inspect 该文档 source=editor 且 nodeCount=0。"""
		structure = self.engine.inspect(self.doc)
		self.assertEqual(structure.source, "editor")
		self.assertEqual(structure.node_count, 0)
		self.assertEqual(structure.wires, [])

	# ── save 原语 ──

	def test_save_roundtrip(self):
		out = os.path.join(self.tmp.name, "rt.gdd")
		result = self.engine.save(self.doc, output=out)
		self.assertTrue(os.path.isfile(out))
		self.assertEqual(result.path, out)
		self.assertGreater(result.bytes_written or 0, 0)
		structure = self.engine.inspect(out)
		self.assertEqual(structure.source, "editor")
		self.assertEqual(structure.node_count, 0)

	# ── exec 原语 ──

	def test_exec_build_and_persist(self):
		"""建点组合技（add_transaction=False）经 exec --save 持久，重开 inspect 可见。"""
		doc = os.path.join(self.tmp.name, "build.gdd")
		self.engine.init(doc)
		messages = msgs.transaction([
			msgs.add_node_via_context_menu(
				700, msgs.definition_identifier("PROTONODE:vector_nodes::generator_nodes::RectangleNode"),
				position=(64, 64),
			),
		])
		result = self.engine.exec_messages(doc, messages, save_to=doc)
		self.assertTrue(result.ok, "exec errors: %s" % result.errors)
		self.assertEqual(result.saved_path, doc)
		structure = self.engine.inspect(doc)
		self.assertEqual(structure.node_count, 1)
		self.assertEqual(structure.nodes[0]["type"], "RectangleNode")

	def test_exec_set_param_value_persists(self):
		"""任务书验收：SetInputValue 级别组合技 --save 重开值不丢（drift 坑防回归）。"""
		doc = os.path.join(self.tmp.name, "param.gdd")
		self.engine.init(doc)
		self.engine.exec_messages(doc, msgs.transaction([
			msgs.add_node_via_context_menu(
				701, msgs.definition_identifier("PROTONODE:vector_nodes::generator_nodes::RectangleNode"),
				position=(32, 32),
			),
		]), save_to=doc)
		# input 1 当前 {"F64": 100.0} → 设 222.5。
		result = self.engine.exec_messages(doc, msgs.transaction([
			msgs.set_input_value(701, 1, msgs.tagged_f64(222.5)),
		]), save_to=doc)
		self.assertTrue(result.ok, "exec errors: %s" % result.errors)
		structure = self.engine.inspect(doc)
		self.assertEqual(structure.param_of(701, "1"), {"F64": 222.5}, "value must survive reopen")

	def test_exec_double_transaction_guard_documented(self):
		"""双层事务实验的防回归锚：内嵌事务（add_transaction=True）在 exec 事务内
		静默丢失（本测试断言这一**已知的坏组合**行为仍在——若未来 Rust 侧修好，
		此测试会翻红提示我们更新 msgs.py 注释与 add_node 默认值）。"""
		doc = os.path.join(self.tmp.name, "guard.gdd")
		self.engine.init(doc)
		inner_transaction = msgs.add_node_via_context_menu(
			702, msgs.definition_identifier("PROTONODE:vector_nodes::generator_nodes::RectangleNode"),
			add_transaction=True,  # 坏组合：与 exec 自动事务叠加
		)
		result = self.engine.exec_messages(doc, msgs.transaction([inner_transaction]), save_to=doc)
		structure = self.engine.inspect(doc)
		# 实测：exec 报 0 错但节点不持久。
		self.assertTrue(result.ok)
		self.assertEqual(structure.node_count, 0, "known drift: inner transaction silently drops the node")

	def test_exec_nonzero_exit_propagates_as_engine_error(self):
		"""坏消息（未知变体）→ Rust 侧非零退出 → EngineError。"""
		doc = os.path.join(self.tmp.name, "bad.gdd")
		self.engine.init(doc)
		with self.assertRaises(EngineError):
			self.engine.exec_messages(doc, [{"Portfolio": {"Document": {"NodeGraph": {"NoSuchVariant": {}}}}}])

	# ── export 原语 ──

	def test_export_svg_and_png_need_artboard(self):
		"""export 需 artboard（实测）；SVG+PNG 产物与摘要契约。"""
		doc = os.path.join(self.tmp.name, "art.gdd")
		self.engine.init(doc)
		self.engine.exec_messages(doc, msgs.transaction([
			msgs.add_node_via_context_menu(
				703, msgs.definition_identifier("PROTONODE:vector_nodes::generator_nodes::RectangleNode"),
				position=(0, 0),
			),
			msgs.new_artboard(704, dimensions=(400.0, 300.0)),
		]), save_to=doc)

		svg_path = os.path.join(self.tmp.name, "out.svg")
		svg = self.engine.export(doc, svg_path)
		self.assertEqual(svg.format, "svg")
		self.assertTrue(os.path.isfile(svg_path))
		self.assertGreater(svg.bytes or 0, 0)
		with open(svg_path, "r", encoding="utf-8") as handle:
			self.assertIn("<svg", handle.read())

		png_path = os.path.join(self.tmp.name, "out.png")
		png = self.engine.export(doc, png_path, width=512)
		self.assertEqual(png.format, "png")
		self.assertEqual(png.width, 512)
		with open(png_path, "rb") as handle:
			self.assertEqual(handle.read(8), b"\x89PNG\r\n\x1a\n")  # PNG magic

	def test_export_without_artboard_fails_loudly(self):
		doc = os.path.join(self.tmp.name, "noart.gdd")
		self.engine.init(doc)
		with self.assertRaises(EngineError):
			self.engine.export(doc, os.path.join(self.tmp.name, "never.svg"))

	# ── inspect 原语 ──

	def test_inspect_reports_wires(self):
		doc = os.path.join(self.tmp.name, "wires.gdd")
		self.engine.init(doc)
		self.engine.exec_messages(doc, msgs.transaction([
			msgs.add_node_via_context_menu(705, msgs.definition_identifier("NETWORK:Merge")),
			msgs.add_node_via_context_menu(706, msgs.definition_identifier("NETWORK:Merge")),
			msgs.create_wire(705, 0, 706, 0),
		]), save_to=doc)
		structure = self.engine.inspect(doc)
		self.assertEqual(structure.node_count, 2)
		self.assertIn({"from": {"node": 705, "output": 0}, "to": {"node": 706, "input": 0}}, structure.wires)


if __name__ == "__main__":
	unittest.main()
