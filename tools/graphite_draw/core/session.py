"""DrawSession：组合技对象（CLI 方案 §〇.2）。

链式攒批：原子操作方法**不立刻执行**，只往批次里攒 Message；``commit()`` 把
攒下的操作 + RunDocumentGraph 收尾打包成 **一个 Message 数组**，一次 exec 注入
= 一次冷启动（CLI 方案 §〇.2：冷启动按组合技段数付，不按操作数付）。

NodeId 由 Python 层自主分配（NodeIds 整型递增，模块 ids.py）：create_layer/add_node
在攒批时就分配并返回 NodeId，后续 connect/set-param 直接引用，commit() 返回
「本段新分配的 name → NodeId」映射作为锚点。

【PY-2 事务语义（实测，防回归）】exec 原语整批自动包
`StartTransaction → 消息 → EndTransaction`（cli/src/exec.rs::execute）——这是**唯一**
的事务层。Python 层不再叠加 AddTransaction（实测叠加会导致改动静默回滚：
AddTransaction 展开的事务对提前提交，外层 EndTransaction 走 Finished 分支不落
registry 快照）。同理 add_node 用 `add_transaction=False`。批次以
`RunDocumentGraph` 收尾触发重编译/渲染（实测无害且必要）。

两种形态：
- 库形态：``DrawSession(doc, engine).add_node(...).connect(...).commit()``
- CLI 形态：graphite_draw.py 的原子子命令（一命令 = 一段组合技 + commit）。

本层不含 JSON 形状细节（msgs.py 的事），不碰进程细节（engine.py 的事）。
"""

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Tuple

from .engine import Engine, ExecResult
from . import msgs
from .ids import NodeIdAllocator


class SessionError(Exception):
	"""DrawSession 用法错误（空段提交、无 engine 提交、引用未分配节点等）。"""


@dataclass
class CommitResult:
	"""commit() 的回执。"""

	exec_result: Optional[ExecResult]      # exec 原语摘要（纯攒批模式为 None）
	node_id_map: Dict[str, int]            # 本段命名锚点 → NodeId
	message_count: int                     # 注入的 Message 总数（含事务壳）
	messages: List[dict]                   # 注入的原始 Message 数组（调试/逃生门）

	@property
	def ok(self) -> bool:
		return self.exec_result is None or self.exec_result.ok


class DrawSession:
	"""一段组合技。生命周期 = 攒批 → commit → 丢弃（换段新开，别复用）。

	Args:
		document: 文档路径（.graphite / .gdd）。None = 纯攒批模式（commit 只回形状，
			不调进程——构造层测试用）。
		engine: Engine 实例。commit() 需要；None 且 document 非 None 时提交报错。
		allocator: 外部注入的 NodeIdAllocator（默认新建，从 1 起步）。
	"""

	def __init__(
		self,
		document: Optional[str] = None,
		engine: Optional[Engine] = None,
		allocator: Optional[NodeIdAllocator] = None,
	):
		self.document = document
		self.engine = engine
		# 注意不能用 `allocator or NodeIdAllocator()`：分配器定义了 __len__，
		# 空分配器 falsy 会被 or 静默换掉。
		self.ids = allocator if allocator is not None else NodeIdAllocator()
		self._operations: List[dict] = []
		self._named: Dict[str, int] = {}  # name → node_id（本段内）

	# ────────────── 内部簿记 ──────────────

	def _name(self, node_id: int, name: Optional[str]) -> int:
		if name:
			if name in self._named:
				raise SessionError("name %r already bound to node %d in this session" % (name, self._named[name]))
			self._named[name] = node_id
		return node_id

	def _require_allocated(self, node: Any, what: str) -> int:
		"""node 允许：已分配的 NodeId（int）/ 本段命名锚点（str）/ 内部占位引用。"""
		if isinstance(node, str):
			if node not in self._named:
				raise SessionError("%s references unknown name %r (not allocated in this session)" % (what, node))
			return self._named[node]
		if isinstance(node, int):
			return node
		raise SessionError("%s must be an int NodeId or a str name, got %r" % (what, node))

	# ────────────── 八件套原子操作（攒批态） ──────────────

	def create_layer(self, name: Optional[str] = None, parent=None, insert_index: int = 0) -> int:
		"""建空图层。parent：None=根；int NodeId / str 锚点。返回新 NodeId。"""
		node_id = self.ids.next()
		parent_id = None if parent is None else self._require_allocated(parent, "create_layer.parent")
		self._operations.append(msgs.create_layer(node_id, parent_node_id=parent_id, insert_index=insert_index))
		return self._name(node_id, name)

	def add_node(self, node_type: str, position: Optional[Tuple[int, int]] = None, name: Optional[str] = None) -> int:
		"""加节点。node_type 用 serialized 定义名（"NETWORK:…"/"PROTONODE:…"）；
		显示名（"Rectangle"）的解析在 PY-2 联调层补（需要编辑器节点注册表）。返回新 NodeId。"""
		node_id = self.ids.next()
		self._operations.append(
			msgs.add_node_via_context_menu(node_id, msgs.definition_identifier(node_type), position=position)
		)
		return self._name(node_id, name)

	def connect(self, from_node, to_node, from_output: int = 0, to_input: int = 0) -> "DrawSession":
		"""连线：from_node:from_output → to_node:to_input。参数可为 NodeId 或锚点名。"""
		src = self._require_allocated(from_node, "connect.from_node")
		dst = self._require_allocated(to_node, "connect.to_node")
		self._operations.append(msgs.create_wire(src, from_output, dst, to_input))
		return self

	def disconnect(self, to_node, to_input: int = 0) -> "DrawSession":
		"""断开 to_node 的第 to_input 路输入。"""
		dst = self._require_allocated(to_node, "disconnect.to_node")
		self._operations.append(msgs.disconnect_input(dst, to_input))
		return self

	def set_param(self, node, input_index: int, value) -> "DrawSession":
		"""设参数。value：msgs.tagged*() 的产物，或形状正确的 TaggedValue dict。"""
		node_id = self._require_allocated(node, "set_param.node")
		self._operations.append(msgs.set_input_value(node_id, input_index, value))
		return self

	def transform(self, layer, matrix) -> "DrawSession":
		"""设图层变换。matrix 用 msgs.daffine2()/translation_matrix() 构造（6 元素）。"""
		layer_id = self._require_allocated(layer, "transform.layer")
		if not (isinstance(matrix, list) and len(matrix) == 6):
			raise SessionError("transform matrix must be a 6-float list (msgs.daffine2), got %r" % (matrix,))
		self._operations.append(msgs.transform_set(layer_id, matrix))
		return self

	def delete_node(self, node) -> "DrawSession":
		"""删节点（DeleteNodes；坑#4：失败只在编辑器日志里，commit 后靠 inspect 验证）。"""
		node_id = self._require_allocated(node, "delete_node.node")
		self._operations.append(msgs.delete_nodes([node_id]))
		return self

	def select(self, nodes) -> "DrawSession":
		"""显式设选区（坑#9：选区是全局状态）。nodes 可混合 NodeId/锚点名。"""
		ids = [self._require_allocated(node, "select.nodes") for node in nodes]
		self._operations.append(msgs.selected_nodes_set(ids))
		return self

	# ────────────── 攒批状态与提交 ──────────────

	@property
	def operations(self) -> List[dict]:
		"""攒下的原子操作 Message（不含 RunGraph 收尾）。返回副本。"""
		return list(self._operations)

	def pending_messages(self) -> List[dict]:
		"""最终注入的 Message 数组：`操作们 → RunDocumentGraph`（无事务壳——exec 自动包，见模块头）。"""
		return msgs.transaction(self._operations)

	def pending_batch(self) -> dict:
		"""Batched 视图（exec 原语实际吃裸数组，这只是给 humans/测试看的打包形状）。"""
		return msgs.batched(self.pending_messages())

	def commit(self, save_to: Optional[str] = None) -> CommitResult:
		"""一次性注入执行（一次冷启动）。save_to 非 None 时段末经编辑器管线存盘 .gdd
		（engine 侧映射为 exec 原语的 ``--save``）。

		纯攒批模式（document=None）只回形状不执行。
		"""
		messages = self.pending_messages()
		if not self._operations:
			raise SessionError("nothing to commit — call at least one atomic operation first")

		exec_result = None
		if self.document is not None:
			if self.engine is None:
				raise SessionError("commit() needs an Engine (DrawSession(doc, engine=…))")
			exec_result = self.engine.exec_messages(self.document, messages, save_to=save_to)

		return CommitResult(
			exec_result=exec_result,
			node_id_map=dict(self._named),
			message_count=len(messages),
			messages=messages,
		)
