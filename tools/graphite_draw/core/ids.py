"""NodeId 分配器（CLI 方案 §〇.2：NodeId 由 Python 层自主分配）。

编辑器原生的 `NodeId::new()` 走 seeded RNG（graphite-cli 进程内行为）；Python 皮这里改用
**整型递增**：同一段组合技脚本 → 同一串 ID，确定性等价，且人肉可读（调试时 1/2/3 对号入座）。

约束：ID 0 是保留值 —— `LayerNodeIdentifier::ROOT_PARENT = NodeId(0)`（document_metadata.rs:297），
任何真实节点不得占用，所以分配器从 1 起步。
"""

class NodeIdAllocator:
	"""整型递增的 NodeId 分配器。线程不安全——组合技攒批是单线程场景，够用。"""

	def __init__(self, start=1):
		# start 不允许 <= 0：0 是 ROOT_PARENT 占位（见模块 docstring）。
		if start < 1:
			raise ValueError("NodeId start must be >= 1 (0 is reserved for ROOT_PARENT)")
		self._next = start
		self._allocated = []  # 分配顺序记录，commit() 回传 id_map 用

	def next(self):
		"""分配下一个 NodeId。"""
		node_id = self._next
		self._next += 1
		self._allocated.append(node_id)
		return node_id

	def allocate(self, count):
		"""一次分配 count 个连续 NodeId，返回列表。"""
		return [self.next() for _ in range(count)]

	@property
	def allocated(self):
		"""已分配的全部 NodeId（分配顺序）。返回副本，防外部误改。"""
		return list(self._allocated)

	def __contains__(self, node_id):
		return node_id in self._allocated

	def __len__(self):
		return len(self._allocated)
