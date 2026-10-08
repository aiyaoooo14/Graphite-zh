"""msgs.py 构造层形状断言：内联期望 JSON 深比较（不碰进程）。

期望形状 = PY-2 实测锚定（cli/docs/message-shapes.md + 本目录 exec 实验）；任何形状
调整应同步改本文件的内联期望值 + msgs.py 构造函数，保持两边对齐。
"""

import json
import unittest

from core import msgs


def assert_json(testcase, actual, expected, note=""):
	"""深比较并给出可读 diff（json 格式化后 assertEqual，失败信息可直接对拍）。"""
	testcase.assertEqual(
		actual, expected,
		"\n--- expected ---\n%s\n--- actual ---\n%s\n%s" % (
			msgs_json(expected), msgs_json(actual), note,
		),
	)


def msgs_json(value):
	return json.dumps(value, ensure_ascii=False, indent=1)


class TestTopLevelNesting(unittest.TestCase):
	"""顶层嵌套：Message → Portfolio → Document → NodeGraph/GraphOperation；Preferences 直连。"""

	def test_node_graph_nesting(self):
		assert_json(self, msgs.node_graph({"a": 1}),
		            {"Portfolio": {"Document": {"NodeGraph": {"a": 1}}}})

	def test_graph_operation_nesting(self):
		assert_json(self, msgs.graph_operation({"b": 2}),
		            {"Portfolio": {"Document": {"GraphOperation": {"b": 2}}}})

	def test_unit_variant_is_bare_string(self):
		assert_json(self, msgs.run_document_graph(),
		            {"Portfolio": {"Document": {"NodeGraph": "RunDocumentGraph"}}})

	def test_preferences_is_single_level(self):
		# ⚠ PY-2 实测：PreferencesMessage 从 Message 顶层直达，无 Portfolio 层。
		assert_json(self, msgs.save_as_gdd(True),
		            {"Preferences": {"SaveAsGdd": {"enabled": True}}})

	def test_batched(self):
		assert_json(self, msgs.batched([msgs.run_document_graph()]),
		            {"Batched": {"messages": [{"Portfolio": {"Document": {"NodeGraph": "RunDocumentGraph"}}}]}})


class TestLeafShapes(unittest.TestCase):
	"""叶子类型 serde 形状（全部实测锚定）。"""

	def test_layer_ref_is_node_id_plus_one(self):
		self.assertEqual(msgs.layer_ref(0), 1)
		self.assertEqual(msgs.layer_ref(7), 8)  # message-shapes.md §2：NodeId(7) → 8

	def test_input_connector_camel_case(self):
		assert_json(self, msgs.input_connector_node(7, 2),
		            {"node": {"nodeId": 7, "inputIndex": 2}})
		assert_json(self, msgs.input_connector_export(0), {"export": 0})

	def test_output_connector_camel_case(self):
		assert_json(self, msgs.output_connector_node(7, 0),
		            {"node": {"nodeId": 7, "outputIndex": 0}})
		assert_json(self, msgs.output_connector_import(1), {"import": 1})

	def test_daffine2_flat_column_major(self):
		assert_json(self, msgs.identity_matrix(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
		assert_json(self, msgs.translation_matrix(10, 20), [1.0, 0.0, 0.0, 1.0, 10.0, 20.0])
		with self.assertRaises(ValueError):
			msgs.daffine2([1, 2, 3])

	def test_tagged_value_measured_variants(self):
		# message-shapes.md §2 实测拼写。
		assert_json(self, msgs.tagged_f64(42.5), {"F64": 42.5})
		assert_json(self, msgs.tagged_bool(True), {"Bool": True})
		assert_json(self, msgs.tagged_string("hi"), {"String": "hi"})
		assert_json(self, msgs.tagged_none(), "None")
		assert_json(self, msgs.tagged_box_corners([1.0, 2.0]), {"BoxCorners": [1.0, 2.0]})

	def test_definition_identifier_strips_prefix(self):
		# PY-2 实测：Network 的 data 是剥前缀裸名（PY-1 曾错带 NETWORK:）。
		assert_json(self, msgs.definition_identifier("NETWORK:Merge"),
		            {"type": "Network", "data": "Merge"})
		assert_json(self, msgs.definition_identifier("PROTONODE:vector_nodes::generator_nodes::RectangleNode"),
		            {"type": "ProtoNode", "data": {"name": "vector_nodes::generator_nodes::RectangleNode"}})
		with self.assertRaises(ValueError):
			msgs.definition_identifier("Rectangle")  # 裸名必须带前缀


class TestBatchIdiom(unittest.TestCase):
	"""PY-2 事务实验结论的形状固化：裸操作数组 + RunGraph 收尾，无 AddTransaction 壳。"""

	def test_transaction_has_no_add_transaction(self):
		ops = [msgs.selected_nodes_set([1])]
		messages = msgs.transaction(ops)
		self.assertEqual(len(messages), 2)
		self.assertEqual(messages[0], ops[0])
		self.assertEqual(messages[-1], {"Portfolio": {"Document": {"NodeGraph": "RunDocumentGraph"}}})
		# 全批不出现 AddTransaction / Batched。
		self.assertNotIn("Batched", [list(m.keys())[0] for m in messages])
		self.assertNotIn("AddTransaction", json.dumps(messages))

	def test_transaction_empty_ops_still_closes_with_run_graph(self):
		self.assertEqual(msgs.transaction([]),
		                 [{"Portfolio": {"Document": {"NodeGraph": "RunDocumentGraph"}}}])


class TestAtomicOperations(unittest.TestCase):
	"""八件套的消息形状（内联完整期望，一个字段都不能多/少/错位）。"""

	def test_create_layer(self):
		assert_json(
			self, msgs.create_layer(3, insert_index=0),
			{"Portfolio": {"Document": {"GraphOperation": {"NewCustomLayer": {
				"id": 3, "nodes": [], "parent": 1, "insert_index": 0,
			}}}}},
		)
		assert_json(
			self, msgs.create_layer(4, parent_node_id=9),
			{"Portfolio": {"Document": {"GraphOperation": {"NewCustomLayer": {
				"id": 4, "nodes": [], "parent": 10, "insert_index": 0,
			}}}}},
		)

	def test_add_node_via_context_menu_defaults_to_no_inner_transaction(self):
		# ⚠ PY-2 实测：add_transaction 必须 False（内嵌事务 + exec 自动事务 = 静默丢失）。
		assert_json(
			self,
			msgs.add_node_via_context_menu(
				2, msgs.definition_identifier("PROTONODE:vector_nodes::generator_nodes::RectangleNode"),
				position=(100, 200),
			),
			{"Portfolio": {"Document": {"NodeGraph": {"CreateNodeFromContextMenu": {
				"node_id": 2,
				"node_type": {"type": "ProtoNode", "data": {"name": "vector_nodes::generator_nodes::RectangleNode"}},
				"xy": [100, 200],
				"add_transaction": False,
			}}}}},
		)
		message = msgs.add_node_via_context_menu(2, msgs.definition_identifier("NETWORK:Merge"))
		self.assertIsNone(message["Portfolio"]["Document"]["NodeGraph"]["CreateNodeFromContextMenu"]["xy"])

	def test_add_nodes_low_level_shape(self):
		assert_json(
			self, msgs.add_nodes([(1, {"document_node": {}, "persistent_node_metadata": {}})]),
			{"Portfolio": {"Document": {"NodeGraph": {"AddNodes": {
				"nodes": [[1, {"document_node": {}, "persistent_node_metadata": {}}]],
				"new_ids": {},
			}}}}},
		)

	def test_create_wire(self):
		assert_json(
			self, msgs.create_wire(1, 0, 2, 0),
			{"Portfolio": {"Document": {"NodeGraph": {"CreateWire": {
				"output_connector": {"node": {"nodeId": 1, "outputIndex": 0}},
				"input_connector": {"node": {"nodeId": 2, "inputIndex": 0}},
			}}}}},
		)

	def test_disconnect_input(self):
		assert_json(
			self, msgs.disconnect_input(2, 1),
			{"Portfolio": {"Document": {"NodeGraph": {"DisconnectInput": {
				"input_connector": {"node": {"nodeId": 2, "inputIndex": 1}},
			}}}}},
		)

	def test_set_input_value(self):
		assert_json(
			self, msgs.set_input_value(2, 0, {"F64": 200.0}),
			{"Portfolio": {"Document": {"NodeGraph": {"SetInputValue": {
				"node_id": 2, "input_index": 0, "value": {"F64": 200.0},
			}}}}},
		)

	def test_transform_set_local(self):
		assert_json(
			self, msgs.transform_set(3, msgs.translation_matrix(10, 20)),
			{"Portfolio": {"Document": {"GraphOperation": {"TransformSet": {
				"layer": 4,                      # NodeId 3 → LayerNodeIdentifier JSON 4
				"transform": [1.0, 0.0, 0.0, 1.0, 10.0, 20.0],
				"transform_in": "Local",         # PY-2 实测：unit 变体裸字符串
				"skip_rerender": False,
			}}}}},
		)

	def test_delete_nodes(self):
		assert_json(
			self, msgs.delete_nodes([5, 6], delete_children=True),
			{"Portfolio": {"Document": {"NodeGraph": {"DeleteNodes": {
				"node_ids": [5, 6], "delete_children": True,
			}}}}},
		)

	def test_selected_nodes_set(self):
		assert_json(
			self, msgs.selected_nodes_set([1, 2]),
			{"Portfolio": {"Document": {"NodeGraph": {"SelectedNodesSet": {"nodes": [1, 2]}}}}},
		)

	def test_new_artboard(self):
		assert_json(
			self, msgs.new_artboard(950, dimensions=(400, 300)),
			{"Portfolio": {"Document": {"GraphOperation": {"NewArtboard": {
				"id": 950,
				"location": [0.0, 0.0],
				"dimensions": [400.0, 300.0],
				"background": {"red": 1.0, "green": 1.0, "blue": 1.0, "alpha": 1.0, "linear": True},
				"clip": True,
			}}}}},
		)


class TestEmptyDocumentTemplate(unittest.TestCase):
	"""init 组合技的 legacy 模板形状（PY-2 实测可被编辑器打开并转存 .gdd）。"""

	def setUp(self):
		self.template = msgs.empty_document_template()

	def test_required_document_fields(self):
		# DocumentMessageHandler 无 default 的字段必须存在（document_message_handler.rs:73+）。
		for key in ("network_interface", "collapsed", "document_ptz", "render_mode",
		            "overlays_visibility_settings", "rulers_visible", "snapping_state",
		            "graph_view_overlay_open", "graph_fade_artwork_percentage"):
			self.assertIn(key, self.template, "missing required field %r" % key)

	def test_network_has_default_artboard_export(self):
		network = self.template["network_interface"]["network"]
		self.assertEqual(network["nodes"], [])
		# 空 exports 会在图扁平化时报 "No input type: []"（实测）——必须有默认导出。
		self.assertEqual(network["exports"], [{
			"Value": {
				"tagged_value": {"TypeDefault": {"List": {"Concrete": {"name": "graphic_types::artboard::Artboard"}}}},
				"exposed": True,
			},
		}])

	def test_network_metadata_present(self):
		# 缺 network_metadata → "missing field network_metadata"（实测）。
		metadata = self.template["network_interface"]["network_metadata"]["persistent_metadata"]
		for key in ("reference", "node_metadata", "pinned_node_order", "previewing", "navigation_metadata"):
			self.assertIn(key, metadata)

	def test_template_is_json_serializable(self):
		json.dumps(self.template)


if __name__ == "__main__":
	unittest.main()
