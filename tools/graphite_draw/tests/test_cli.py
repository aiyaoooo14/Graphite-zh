import sys
from pathlib import Path

# unittest discover 从任意 cwd 跑都能导入 core/*（本文件相对 tools/graphite_draw 上溯一级）。
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

"""CLI 入口的参数解析测试（不调进程：只验证 argparse 路由与引用解析）。"""

import unittest

from graphite_draw import _node_ref, build_parser


class TestParserRouting(unittest.TestCase):
	def test_primitives_parse(self):
		args = build_parser().parse_args(["save", "doc.gdd", "-o", "out.gdd"])
		self.assertEqual((args.command, args.document, args.output), ("save", "doc.gdd", "out.gdd"))

		args = build_parser().parse_args(["inspect", "doc.gdd", "--pretty"])
		self.assertEqual((args.command, args.pretty), ("inspect", True))

		# PY-2 契约：--json-string / --file 二选一必填；--save（不再是 --save-to）。
		args = build_parser().parse_args(["exec", "doc.gdd", "--json-string", "[]", "--save", "o.gdd"])
		self.assertEqual((args.json_string, args.save), ("[]", "o.gdd"))
		args = build_parser().parse_args(["exec", "doc.gdd", "--file", "msgs.json"])
		self.assertEqual(args.file, "msgs.json")
		with self.assertRaises(SystemExit):
			build_parser().parse_args(["exec", "doc.gdd"])  # 无消息源 → 拒绝

		args = build_parser().parse_args(["export", "doc.gdd", "-o", "x.png",
		                                  "--width", "800", "--scale", "2.0", "--transparent"])
		self.assertEqual((args.output, args.width, args.scale, args.transparent), ("x.png", 800, 2.0, True))

	def test_init_parse(self):
		args = build_parser().parse_args(["init", "fresh.gdd"])
		self.assertEqual((args.command, args.output), ("init", "fresh.gdd"))

	def test_atomic_commands_parse(self):
		args = build_parser().parse_args(
			["add-node", "doc.gdd", "PROTONODE:vector_nodes::generator_nodes::RectangleNode",
			 "--position", "10", "20", "--name", "rect", "--save", "out.gdd"])
		self.assertEqual(args.node_type, "PROTONODE:vector_nodes::generator_nodes::RectangleNode")
		self.assertEqual(args.position, [10, 20])
		self.assertEqual(args.name, "rect")
		self.assertEqual(args.save, "out.gdd")

		args = build_parser().parse_args(["connect", "d", "1", "2", "--from-output", "1", "--to-input", "2"])
		self.assertEqual((args.from_node, args.to_node, args.from_output, args.to_input), ("1", "2", 1, 2))

		args = build_parser().parse_args(
			["transform", "d", "3", "--matrix", "1", "0", "0", "1", "10", "20"])
		self.assertEqual(args.matrix, [1.0, 0.0, 0.0, 1.0, 10.0, 20.0])

		args = build_parser().parse_args(
			["set-param", "d", "5", "--input", "1", "--value", '{"F64": 200.0}'])
		self.assertEqual(args.value, '{"F64": 200.0}')

	def test_global_options_before_subcommand(self):
		args = build_parser().parse_args(
			["--engine", "/x/graphite-cli", "--timeout", "30", "save", "d.gdd"])
		self.assertEqual(args.engine, "/x/graphite-cli")
		self.assertEqual(args.timeout, 30.0)

	def test_engine_and_file_are_exclusive_in_exec(self):
		with self.assertRaises(SystemExit):
			build_parser().parse_args(["exec", "d", "--json-string", "[]", "--file", "f.json"])


class TestNodeRef(unittest.TestCase):
	def test_digits_become_ids(self):
		self.assertEqual(_node_ref("41"), 41)

	def test_names_pass_through(self):
		self.assertEqual(_node_ref("rect"), "rect")


if __name__ == "__main__":
	unittest.main()
