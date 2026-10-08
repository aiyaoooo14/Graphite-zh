import sys
from pathlib import Path

# unittest discover 从任意 cwd 跑都能导入 core/*（本文件相对 tools/graphite_draw 上溯一级）。
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

"""NodeId 分配器测试（纯构造层，不碰进程）。"""

import unittest

from core.ids import NodeIdAllocator


class TestNodeIdAllocator(unittest.TestCase):
	def test_increments_from_one(self):
		alloc = NodeIdAllocator()
		self.assertEqual([alloc.next(), alloc.next(), alloc.next()], [1, 2, 3])

	def test_zero_is_reserved(self):
		# 0 = ROOT_PARENT 占位（ids.py docstring / document_metadata.rs:297）。
		with self.assertRaises(ValueError):
			NodeIdAllocator(start=0)

	def test_allocate_batch_is_contiguous(self):
		alloc = NodeIdAllocator()
		self.assertEqual(alloc.allocate(3), [1, 2, 3])
		self.assertEqual(alloc.next(), 4)

	def test_allocated_records_order(self):
		alloc = NodeIdAllocator()
		alloc.next()
		alloc.allocate(2)
		self.assertEqual(alloc.allocated, [1, 2, 3])
		# 副本防误改。
		alloc.allocated.clear()
		self.assertEqual(len(alloc), 3)

	def test_contains(self):
		alloc = NodeIdAllocator()
		alloc.next()
		self.assertIn(1, alloc)
		self.assertNotIn(2, alloc)


if __name__ == "__main__":
	unittest.main()
