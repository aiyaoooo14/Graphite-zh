#!/usr/bin/env python3
"""graphite_draw — Graphite 编辑器的 Python 命令皮（CLI 方案 §〇 形态 v2，PY-2 联调态）。

用法一：CLI（每条命令 = 一段组合技 = 一次冷启动）::

    python3 tools/graphite_draw/graphite_draw.py init out.gdd
    python3 tools/graphite_draw/graphite_draw.py add-node out.gdd \\
        "PROTONODE:vector_nodes::generator_nodes::RectangleNode" \\
        --position 100 200 --save out2.gdd
    python3 tools/graphite_draw/graphite_draw.py inspect out.gdd --pretty

用法二：库（agent skill 層推荐）::

    import sys; sys.path.insert(0, "tools/graphite_draw")
    from core.engine import Engine
    from core.session import DrawSession
    from core import msgs

    engine = Engine()
    engine.init("doc.gdd")                     # 空文档（legacy 模板 → 编辑器 .gdd）
    s = DrawSession("doc.gdd", engine)
    a = s.create_layer(name="bg")
    b = s.add_node("PROTONODE:vector_nodes::generator_nodes::RectangleNode", name="rect")
    s.connect(a, b).set_param("rect", 1, msgs.tagged_f64(200.0))
    result = s.commit(save_to="out.gdd")       # 一次冷启动执行全部

原子命令 = 原语组合惯用法：底层全部走 ``exec`` 原语注入 Message 数组（exec 整批自动
包事务，Python 层不叠加——实测叠加会静默回滚），NodeId 由 Python 层自主分配。
serde JSON 形状集中在 core/msgs.py（形状契约表 + 实测证据）。
"""

import argparse
import json
import sys
from pathlib import Path

# 让 `python3 tools/graphite_draw/graphite_draw.py` 直接可跑（无包安装）。
sys.path.insert(0, str(Path(__file__).resolve().parent))

from core.engine import Engine, EngineError, default_timeout  # noqa: E402
from core.session import DrawSession, SessionError  # noqa: E402
from core import msgs  # noqa: E402


# ────────────────────────── 参数解析 ──────────────────────────

def build_parser() -> argparse.ArgumentParser:
	parser = argparse.ArgumentParser(
		prog="graphite_draw",
		description="Graphite Python command skin over the graphite-cli engine (4 primitives).",
	)
	parser.add_argument("--engine", default=None, metavar="PATH",
	                    help="graphite-cli binary path (default: $GRAPHITE_CLI_BIN, then target/{debug,release} probing)")
	parser.add_argument("--resource-cache", default=None, metavar="DIR",
	                    help="editor mmap resource storage dir (passed through to the engine)")
	parser.add_argument("--log-filter", default=None, metavar="FILTER",
	                    help="env_logger filter for the engine (default: engine's own warn)")
	parser.add_argument("--timeout", type=float, default=None, metavar="SEC",
	                    help="per-invocation subprocess timeout (default: 120s)")
	sub = parser.add_subparsers(dest="command", required=True)

	# ---- 原语透传四件 ----
	p = sub.add_parser("save", help="primitive: re-save a document as .gdd")
	p.add_argument("document")
	p.add_argument("-o", "--output", default=None)

	p = sub.add_parser("inspect", help="primitive: print document structure JSON")
	p.add_argument("document")
	p.add_argument("--pretty", action="store_true")

	p = sub.add_parser("exec", help="primitive: inject a Message JSON array (escape hatch)")
	p.add_argument("document")
	source = p.add_mutually_exclusive_group(required=True)
	source.add_argument("--json-string", default=None, help="literal Message JSON array")
	source.add_argument("--file", default=None, help="JSON file ('-' = stdin; a skin-side convenience)")
	p.add_argument("--save", default=None, help="save as .gdd after the batch")

	p = sub.add_parser("export", help="primitive: editor-chain export (.svg/.png; document needs an artboard)")
	p.add_argument("document")
	p.add_argument("-o", "--output", required=True)
	p.add_argument("--width", type=int, default=None)
	p.add_argument("--height", type=int, default=None)
	p.add_argument("--scale", type=float, default=None)
	p.add_argument("--transparent", action="store_true")

	# ---- init：空文档（写 legacy 模板 → save 原语转换为编辑器 .gdd）----
	p = sub.add_parser("init", help="create an empty editor-backed .gdd document")
	p.add_argument("output", help="path of the .gdd to create")

	# ---- 原子命令八件套（= exec 原语组合，见 core/session.py）----
	# 共同尾参：--save（段末存盘，对齐 Rust exec 原语的命名）。
	def atomic(p):
		p.add_argument("document")
		p.add_argument("--save", default=None, help="save as .gdd after this combination segment")

	p = sub.add_parser("create-layer", help="atomic: new empty layer at root")
	atomic(p)
	p.add_argument("--name", default=None, help="anchor name for the new node (commit() id map key)")
	p.add_argument("--parent", default=None, help="parent node id or anchor name (default: root)")
	p.add_argument("--insert-index", type=int, default=0)

	p = sub.add_parser("add-node", help="atomic: add a node (serialized type: NETWORK:… / PROTONODE:…)")
	atomic(p)
	p.add_argument("node_type")
	p.add_argument("--position", nargs=2, type=int, default=None, metavar=("X", "Y"))
	p.add_argument("--name", default=None)

	p = sub.add_parser("connect", help="atomic: wire FROM:from-output → TO:to-input")
	atomic(p)
	p.add_argument("from_node")
	p.add_argument("to_node")
	p.add_argument("--from-output", type=int, default=0)
	p.add_argument("--to-input", type=int, default=0)

	p = sub.add_parser("disconnect", help="atomic: remove the wire into TO:to-input")
	atomic(p)
	p.add_argument("to_node")
	p.add_argument("--to-input", type=int, default=0)

	p = sub.add_parser("set-param", help="atomic: set a node input to a TaggedValue JSON (e.g. '{\"F64\": 200.0}'; same variant as the current value)")
	atomic(p)
	p.add_argument("node")
	p.add_argument("--input", type=int, default=0)
	p.add_argument("--value", required=True, help="TaggedValue JSON (externally tagged)")

	p = sub.add_parser("transform", help="atomic: set a layer's DAffine2 (6 column-major floats)")
	atomic(p)
	p.add_argument("layer")
	p.add_argument("--matrix", nargs=6, type=float, required=True, metavar=("C0X", "C0Y", "C1X", "C1Y", "C2X", "C2Y"))

	p = sub.add_parser("delete-node", help="atomic: delete a node")
	atomic(p)
	p.add_argument("node")

	p = sub.add_parser("select", help="atomic: explicitly set the selection")
	atomic(p)
	p.add_argument("nodes", nargs="+")

	return parser


def _node_ref(text: str):
	"""CLI 的节点引用：纯数字 → NodeId(int)，否则 → 锚点名(str)。"""
	text = text.strip()
	if text.isdigit():
		return int(text)
	return text


# ────────────────────────── 命令实现 ──────────────────────────

def _emit(payload: dict) -> None:
	"""统一 stdout：一行 JSON 摘要（agent 契约：新 NodeId/结果机器可读）。"""
	print(json.dumps(payload, ensure_ascii=False))


def _atomic_commit(args, session: DrawSession, extra: dict) -> int:
	result = session.commit(save_to=args.save)
	payload = {
		"command": args.command,
		"document": args.document,
		"node_id_map": result.node_id_map,
		"message_count": result.message_count,
	}
	if args.save:
		payload["saved_path"] = result.exec_result.saved_path if result.exec_result else args.save
	payload.update(extra)
	if result.exec_result is not None:
		payload["exec"] = {
			"error_count": result.exec_result.error_count,
			"events": result.exec_result.events,
		}
		if result.exec_result.saved_path:
			payload["exec"]["saved_path"] = result.exec_result.saved_path
		if not result.exec_result.ok:
			payload["error"] = "exec finished with %d error(s)" % result.exec_result.error_count
			_emit(payload)
			return 1
	_emit(payload)
	return 0


def _session(args, engine: Engine) -> DrawSession:
	return DrawSession(args.document, engine)


def cmd_init(args, engine: Engine) -> int:
	path = engine.init(args.output)
	_emit({"command": "init", "path": path})
	return 0


def cmd_save(args, engine: Engine) -> int:
	result = engine.save(args.document, output=args.output)
	_emit({"command": "save", "path": result.path, "bytes": result.bytes_written, "raw": result.raw})
	return 0


def cmd_inspect(args, engine: Engine) -> int:
	structure = engine.inspect(args.document)
	if args.pretty:
		print(json.dumps(structure.raw, ensure_ascii=False, indent=2))
	else:
		_emit(structure.raw)
	return 0


def cmd_exec(args, engine: Engine) -> int:
	if args.json_string is not None:
		payload = args.json_string
	elif args.file == "-":
		# 皮侧扩展：stdin 读入后经 --file 传递（Rust exec 终态无 stdin 缺省）。
		payload = sys.stdin.read()
	else:
		with open(args.file, "r", encoding="utf-8") as handle:
			payload = handle.read()
	messages = json.loads(payload)  # 形状校验交给 Rust 侧 serde；这里只保证是 JSON
	result = engine.exec_messages(args.document, messages, save_to=args.save)
	summary = {"error_count": result.error_count, "events": result.events}
	if result.saved_path:
		summary["saved_path"] = result.saved_path
	_emit({"command": "exec", "exec": summary})
	return 0 if result.ok else 1


def cmd_export(args, engine: Engine) -> int:
	result = engine.export(
		args.document, args.output,
		width=args.width, height=args.height, scale=args.scale, transparent=args.transparent,
	)
	_emit({"command": "export", **result.raw})
	return 0


def cmd_create_layer(args, engine: Engine) -> int:
	session = _session(args, engine)
	layer = session.create_layer(
		name=args.name,
		parent=_node_ref(args.parent) if args.parent else None,
		insert_index=args.insert_index,
	)
	return _atomic_commit(args, session, {"created_layer": layer})


def cmd_add_node(args, engine: Engine) -> int:
	session = _session(args, engine)
	node = session.add_node(
		args.node_type,
		position=tuple(args.position) if args.position else None,
		name=args.name,
	)
	return _atomic_commit(args, session, {"new_node_id": node})


def cmd_connect(args, engine: Engine) -> int:
	session = _session(args, engine)
	session.connect(_node_ref(args.from_node), _node_ref(args.to_node),
	                from_output=args.from_output, to_input=args.to_input)
	return _atomic_commit(args, session, {
		"connected": {"from": args.from_node, "to": args.to_node,
		              "from_output": args.from_output, "to_input": args.to_input},
	})


def cmd_disconnect(args, engine: Engine) -> int:
	session = _session(args, engine)
	session.disconnect(_node_ref(args.to_node), to_input=args.to_input)
	return _atomic_commit(args, session, {"disconnected": {"to": args.to_node, "to_input": args.to_input}})


def cmd_set_param(args, engine: Engine) -> int:
	try:
		value = json.loads(args.value)
	except json.JSONDecodeError as error:
		print("error: --value must be TaggedValue JSON (externally tagged, e.g. "
		      "'{\"F64\": 200.0}'): %s" % error, file=sys.stderr)
		return 2
	session = _session(args, engine)
	session.set_param(_node_ref(args.node), args.input, value)
	return _atomic_commit(args, session, {"set_param": {"node": args.node, "input": args.input}})


def cmd_transform(args, engine: Engine) -> int:
	session = _session(args, engine)
	session.transform(_node_ref(args.layer), msgs.daffine2(args.matrix))
	return _atomic_commit(args, session, {"transformed": {"layer": args.layer}})


def cmd_delete_node(args, engine: Engine) -> int:
	session = _session(args, engine)
	session.delete_node(_node_ref(args.node))
	return _atomic_commit(args, session, {"deleted": args.node})


def cmd_select(args, engine: Engine) -> int:
	session = _session(args, engine)
	session.select([_node_ref(node) for node in args.nodes])
	return _atomic_commit(args, session, {"selected": args.nodes})


COMMANDS = {
	"init": cmd_init,
	"save": cmd_save,
	"inspect": cmd_inspect,
	"exec": cmd_exec,
	"export": cmd_export,
	"create-layer": cmd_create_layer,
	"add-node": cmd_add_node,
	"connect": cmd_connect,
	"disconnect": cmd_disconnect,
	"set-param": cmd_set_param,
	"transform": cmd_transform,
	"delete-node": cmd_delete_node,
	"select": cmd_select,
}


def main(argv=None) -> int:
	args = build_parser().parse_args(argv)
	engine = Engine(
		binary=args.engine,
		resource_cache=args.resource_cache,
		log_filter=args.log_filter,
		timeout=args.timeout if args.timeout is not None else default_timeout(),
	)
	try:
		return COMMANDS[args.command](args, engine)
	except (EngineError, SessionError) as error:
		print("error: %s" % error, file=sys.stderr)
		return 1
	except json.JSONDecodeError as error:
		print("error: invalid JSON input: %s" % error, file=sys.stderr)
		return 2
	except FileNotFoundError as error:
		print("error: %s" % error, file=sys.stderr)
		return 2


if __name__ == "__main__":
	sys.exit(main())
