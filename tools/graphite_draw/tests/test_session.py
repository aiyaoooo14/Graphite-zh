"""DrawSession 组合技攒批逻辑测试（不调进程：纯构造 + 形状断言）。

验收用例「建3节点连2线设1参」在 TestCombinationSample：字面拆解 3 节点+2 线+1 参 =
6 操作；PY-2 起批次形态 = 操作们 + RunDocumentGraph 收尾（**无 AddTransaction 壳**，
exec 自动包事务——PY-2 实验结论），故消息总数 = 6+1 = 7。
真进程联调（值持久性/导出）见 test_engine.py 的 TestIntegration。
"""

import unittest

from core import msgs
from core.ids import NodeIdAllocator
from core.session import DrawSession, SessionError


class TestBatching(unittest.TestCase):
	"""攒批：操作不立刻执行，commit 时一次成型。"""

	def test_operations_accumulate_without_execution(self):
		session = DrawSession()  # 无 document = 纯攒批
		a = session.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode")
		b = session.add_node("NETWORK:Merge")
		self.assertEqual((a, b), (1, 2))           # NodeId 整型递增，自主分配
		self.assertEqual(len(session.operations), 2)

	def test_pending_messages_close_with_run_graph(self):
		session = DrawSession()
		session.add_node("NETWORK:Merge")
		messages = session.pending_messages()
		self.assertEqual(len(messages), 1 + 1)              # 操作 + RunGraph
		self.assertEqual(messages[-1], {"Portfolio": {"Document": {"NodeGraph": "RunDocumentGraph"}}})
		# 无事务壳（exec 自动包，PY-2 实验结论）。
		self.assertNotIn("AddTransaction", __import__("json").dumps(messages))

	def test_commit_without_operations_rejects(self):
		session = DrawSession()
		with self.assertRaises(SessionError):
			session.commit()

	def test_commit_in_batch_only_mode_returns_shape(self):
		session = DrawSession()  # document=None → 不需要 engine，不调进程
		session.add_node("NETWORK:Merge", name="n")
		result = session.commit()
		self.assertIsNone(result.exec_result)
		self.assertTrue(result.ok)
		self.assertEqual(result.node_id_map, {"n": 1})
		self.assertEqual(result.message_count, 2)

	def test_commit_without_engine_rejects_when_document_set(self):
		session = DrawSession("doc.gdd", engine=None)
		session.add_node("NETWORK:Merge")
		with self.assertRaises(SessionError):
			session.commit()


class TestNameAnchors(unittest.TestCase):
	"""命名锚点：commit() 的 node_id_map 是后续命令的接驳点。"""

	def test_names_bind_to_allocated_ids(self):
		session = DrawSession()
		session.create_layer(name="bg")
		rect = session.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode", name="rect")
		session.connect("bg", "rect")                     # 锚点名引用
		session.set_param("rect", 1, msgs.tagged_f64(100.0))
		result = session.commit()
		self.assertEqual(result.node_id_map, {"bg": 1, "rect": 2})
		self.assertEqual(rect, 2)

	def test_duplicate_name_rejects(self):
		session = DrawSession()
		session.add_node("NETWORK:Merge", name="x")
		with self.assertRaises(SessionError):
			session.add_node("NETWORK:Merge", name="x")

	def test_unknown_name_rejects(self):
		session = DrawSession()
		with self.assertRaises(SessionError):
			session.connect("ghost", 1)

	def test_literal_ids_pass_through(self):
		# 已有文档节点（inspect 得到的 id）直接用数字引用，不要求本段分配过。
		session = DrawSession()
		session.connect(41, 42, from_output=1, to_input=2)
		self.assertEqual(len(session.operations), 1)

	def test_bad_reference_type_rejects(self):
		session = DrawSession()
		with self.assertRaises(SessionError):
			session.connect(1.5, 2)


class TestCombinationSample(unittest.TestCase):
	"""验收组合技：「建3节点连2线设1参」——Message 数组 JSON 结构完整性。"""

	def test_three_nodes_two_wires_one_param(self):
		session = DrawSession()
		# 3 个节点：1 个图层 + 2 个节点（Rectangle → Blur 链，挂到图层）。
		layer = session.create_layer(name="layer")
		rect = session.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode",
		                        position=(0, 0), name="rect")
		blur = session.add_node("PROTONODE:vector_nodes::filter_nodes::BlurNode",
		                        position=(200, 0), name="blur")
		# 连 2 线：rect 出 → blur 入；blur 出 → 图层入。
		session.connect(rect, blur)
		session.connect(blur, layer, to_input=1)
		# 设 1 参。
		session.set_param(blur, 0, msgs.tagged_f64(20.0))

		messages = session.pending_messages()
		# PY-2 形态：6 操作 + RunGraph 收尾 = 7（无 AddTransaction 壳，exec 自动包事务）。
		self.assertEqual(len(messages), 6 + 1)

		# 收尾必须是 RunGraph；全程无 AddTransaction。
		self.assertEqual(messages[-1], {"Portfolio": {"Document": {"NodeGraph": "RunDocumentGraph"}}})
		self.assertNotIn("AddTransaction", __import__("json").dumps(messages))

		# 6 个操作的形状逐条核对（序 = 攒批序）。
		node_graph_of = lambda m: m["Portfolio"]["Document"]["NodeGraph"]
		graph_operation_of = lambda m: m["Portfolio"]["Document"]["GraphOperation"]

		self.assertEqual(graph_operation_of(messages[0]), {"NewCustomLayer": {
			"id": 1, "nodes": [], "parent": 1, "insert_index": 0,   # create_layer
		}})
		self.assertEqual(node_graph_of(messages[1])["CreateNodeFromContextMenu"]["node_id"], 2)   # add_node rect
		self.assertEqual(node_graph_of(messages[2])["CreateNodeFromContextMenu"]["node_id"], 3)   # add_node blur
		self.assertFalse(node_graph_of(messages[1])["CreateNodeFromContextMenu"]["add_transaction"])
		self.assertEqual(node_graph_of(messages[3]), {"CreateWire": {   # connect rect→blur
			"output_connector": {"node": {"nodeId": 2, "outputIndex": 0}},
			"input_connector": {"node": {"nodeId": 3, "inputIndex": 0}},
		}})
		self.assertEqual(node_graph_of(messages[4]), {"CreateWire": {   # connect blur→layer:1
			"output_connector": {"node": {"nodeId": 3, "outputIndex": 0}},
			"input_connector": {"node": {"nodeId": 1, "inputIndex": 1}},
		}})
		self.assertEqual(node_graph_of(messages[5]), {"SetInputValue": {   # set_param blur:0
			"node_id": 3, "input_index": 0, "value": {"F64": 20.0},
		}})

		# commit（纯攒批模式）回执。
		result = session.commit()
		self.assertEqual(result.node_id_map, {"layer": 1, "rect": 2, "blur": 3})
		self.assertEqual(result.message_count, 7)


class TestOperationMapping(unittest.TestCase):
	"""其余原子操作 → Message 的映射。"""

	def test_disconnect(self):
		session = DrawSession()
		session.disconnect(9, to_input=2)
		self.assertEqual(session.operations[0], msgs.disconnect_input(9, 2))

	def test_transform_requires_6_element_list(self):
		session = DrawSession()
		with self.assertRaises(SessionError):
			session.transform(1, [1.0, 0.0])  # 缺元素
		session.transform(1, msgs.identity_matrix())
		self.assertEqual(session.operations[0], msgs.transform_set(1, msgs.identity_matrix()))

	def test_delete_node(self):
		session = DrawSession()
		session.delete_node(7)
		self.assertEqual(session.operations[0], msgs.delete_nodes([7], delete_children=True))

	def test_select_mixed_refs(self):
		session = DrawSession()
		first = session.add_node("NETWORK:Merge", name="a")
		session.select([first, 99])
		self.assertEqual(session.operations[-1], msgs.selected_nodes_set([1, 99]))

	def test_injected_allocator_is_shared(self):
		alloc = NodeIdAllocator(start=100)
		session = DrawSession(allocator=alloc)
		node = session.add_node("NETWORK:Merge")
		self.assertEqual(node, 100)


if __name__ == "__main__":
	unittest.main()
