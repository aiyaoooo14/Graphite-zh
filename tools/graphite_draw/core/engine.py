"""Engine：graphite-cli 四原语的 subprocess 封装（CLI 方案 §〇.1）。

四原语（Rust API 引擎，一锤子进程：启动→打开→操作→存盘→退出）。
**PY-2 已对齐 cli/ 终态**（2026-10-08 重锚定）：

- ``save <doc> [--output/-o <out>]``   转存 .gdd（stdout 文本行 ``Saved X -> Y (N bytes)``）
- ``inspect <doc> [--pretty]``         → stdout JSON
  ``{source, nodeCount, nodes, wires, layers}``（inspect.rs 契约；
  source="editor"|"registry"，compiler-stack .gdd 走 registry 读路径）
- ``exec <doc> (--json-string <s> | --file <f>) [--save <out>]``
  注入 Message JSON 数组（两源二选一，**无 stdin 缺省**）→ stdout 摘要
  ``{events, error_count, saved_path?}``（saved_path 仅 --save 时出现）。
  exec 整批自动包事务（StartTransaction → 消息 → EndTransaction，exec.rs::execute）。
  error_count>0 时命令退出码非零。
- ``export <doc> -o <out> [--width --height --scale --transparent]``
  编辑器链导出，扩展名选格式（.svg/.png）；stdout 摘要
  ``{saved_path, bytes, format, width, height, scale}``。
  ⚠ 实测：文档需含 artboard，否则编辑器渲染链弹 Error（见 msgs.new_artboard）。

本层职责：二进制定位、进程超时、stderr→异常传播、stdout 解析成 Python 对象。
不含任何 Message 构造逻辑（msgs.py/session.py 的事）。
"""

import json
import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional

from . import msgs as _msgs


class EngineError(Exception):
	"""graphite-cli 调用失败（找不到二进制 / 非零退出 / 超时 / 输出不可解析）。"""


@dataclass
class SaveResult:
	"""``save`` 的结果（stdout 文本行宽松解析）。"""

	raw: str                    # 原始 stdout
	path: Optional[str] = None  # 落盘路径（解析失败时 None）
	bytes_written: Optional[int] = None
	document: Optional[str] = None


@dataclass
class ExecResult:
	"""``exec`` 的结果。字段 = cli/src/exec.rs::ExecSummary 的 stdout 契约（PY-2 终态）。"""

	raw: dict = field(default_factory=dict)  # 原始 stdout JSON
	events: List[dict] = field(default_factory=list)  # 拦截的前端副效应（kind: save_document/save_file/export_image/dialog/suppressed/error）
	error_count: int = 0           # 派发失败数 + 对话框数
	saved_path: Optional[str] = None  # 仅 --save 时出现

	@property
	def errors(self) -> List[dict]:
		"""events 里的 error 项（含出错的 message_index 与 detail）。"""
		return [event for event in self.events if event.get("kind") == "error"]

	@property
	def ok(self) -> bool:
		return self.error_count == 0

	@classmethod
	def from_stdout(cls, stdout: str) -> "ExecResult":
		try:
			raw = json.loads(stdout)
		except json.JSONDecodeError as error:
			raise EngineError("exec summary is not valid JSON: %s\nstdout: %r" % (error, stdout[:400])) from error
		return cls(
			raw=raw,
			events=list(raw.get("events", [])),
			error_count=int(raw.get("error_count", 0)),
			saved_path=raw.get("saved_path"),
		)


@dataclass
class InspectResult:
	"""``inspect`` 的结构化视图（inspect.rs 输出契约，PY-2 终态）。"""

	raw: dict = field(default_factory=dict)
	source: str = "editor"          # "editor" | "registry"
	node_count: int = 0
	nodes: List[dict] = field(default_factory=list)   # {id,type,identifier,display_name,is_layer,position,params}
	wires: List[dict] = field(default_factory=list)   # {from:{node,output}, to:{node,input}}
	layers: List[dict] = field(default_factory=list)  # {id,name,children}

	@classmethod
	def from_stdout(cls, stdout: str) -> "InspectResult":
		try:
			raw = json.loads(stdout)
		except json.JSONDecodeError as error:
			raise EngineError("inspect output is not valid JSON: %s\nstdout: %r" % (error, stdout[:400])) from error
		return cls(
			raw=raw,
			source=str(raw.get("source", "editor")),
			node_count=int(raw.get("nodeCount", 0)),
			nodes=list(raw.get("nodes", [])),
			wires=list(raw.get("wires", [])),
			layers=list(raw.get("layers", [])),
		)

	def param_of(self, node_id: int, input_index):
		"""取节点某输入的当前值（无则 None）；params 形如 {"1": {"F64": 100.0}}。"""
		for node in self.nodes:
			if node.get("id") == node_id:
				return node.get("params", {}).get(str(input_index))
		return None


@dataclass
class ExportResult:
	"""``export`` 的结果（export.rs::ExportSummary stdout 契约，PY-2 终态）。"""

	raw: dict = field(default_factory=dict)
	saved_path: Optional[str] = None
	bytes: Optional[int] = None
	format: Optional[str] = None    # "svg" | "png"
	width: Optional[int] = None
	height: Optional[int] = None
	scale: Optional[float] = None

	@classmethod
	def from_stdout(cls, stdout: str) -> "ExportResult":
		try:
			raw = json.loads(stdout)
		except json.JSONDecodeError as error:
			raise EngineError("export summary is not valid JSON: %s\nstdout: %r" % (error, stdout[:400])) from error
		return cls(
			raw=raw,
			saved_path=raw.get("saved_path"),
			bytes=raw.get("bytes"),
			format=raw.get("format"),
			width=raw.get("width"),
			height=raw.get("height"),
			scale=raw.get("scale"),
		)


def default_timeout() -> float:
	"""默认进程超时：单次冷启动约 1~3s（CLI 方案 §一.2），export 含 wgpu 光栅化给足余量。"""
	return 120.0


class Engine:
	"""graphite-cli 二进制的薄封装。可复用；线程安全无承诺（组合技攒批是单线程场景）。"""

	def __init__(
		self,
		binary: Optional[str] = None,
		resource_cache: Optional[str] = None,
		log_filter: Optional[str] = None,
		timeout: float = default_timeout(),
	):
		self._binary = binary
		self.resource_cache = resource_cache
		# PY-2：默认不传 --log-filter，让 Rust 侧用自身默认值（warn）；None = 不传。
		self.log_filter = log_filter
		self.timeout = timeout

	# ────────────── 二进制定位 ──────────────

	def locate_binary(self) -> str:
		"""按优先级探测 graphite-cli：

		1. 构造参数 binary；
		2. 环境变量 ``GRAPHITE_CLI_BIN``；
		3. 仓库根 ``target/{debug,release}/graphite-cli``（cwd 与本文件向上推导双探测）；
		4. PATH 上的 ``graphite-cli``。

		显式指定（参数/环境变量）但无效 → 直接报错，不静默 fallback（防指鹿为马）。
		"""
		explicit = self._binary or os.environ.get("GRAPHITE_CLI_BIN")
		if explicit:
			if os.path.isfile(explicit) and os.access(explicit, os.X_OK):
				return explicit
			raise EngineError("explicit graphite-cli binary %r not found or not executable" % explicit)

		for root in self._repo_roots():
			for profile in ("debug", "release"):
				candidate = root / "target" / profile / "graphite-cli"
				if candidate.is_file() and os.access(str(candidate), os.X_OK):
					return str(candidate)
		from_path = shutil.which("graphite-cli")
		if from_path:
			return from_path
		raise EngineError(
			"graphite-cli binary not found — build it (cargo build -p graphite-cli), "
			"set GRAPHITE_CLI_BIN, or pass Engine(binary=…)"
		)

	@staticmethod
	def _repo_roots():
		"""候选仓库根：当前工作目录 + 本文件的全部上级（覆盖任意 cwd）。"""
		roots = {Path.cwd()}
		for parent in Path(__file__).resolve().parents:
			roots.add(parent)
		return roots

	# ────────────── 进程执行 ──────────────

	def _run(self, command_args: List[str], input_text: Optional[str] = None) -> str:
		"""拼全局参数 → 子进程 → 校验退出码 → 返回 stdout。"""
		binary = self.locate_binary()
		args = [binary]
		# clap 全局参数（非 global=true）必须置于子命令之前。
		if self.resource_cache:
			args += ["--resource-cache", self.resource_cache]
		if self.log_filter:
			args += ["--log-filter", self.log_filter]
		args += command_args

		try:
			proc = subprocess.run(
				args,
				input=input_text,
				capture_output=True,
				text=True,
				timeout=self.timeout,
			)
		except subprocess.TimeoutExpired as error:
			raise EngineError("graphite-cli timed out after %ss: %s" % (self.timeout, " ".join(args))) from error
		except OSError as error:
			raise EngineError("failed to spawn graphite-cli (%s): %s" % (binary, error)) from error

		if proc.returncode != 0:
			raise EngineError(
				"graphite-cli exited %d: %s\nstderr: %s" % (proc.returncode, " ".join(args), proc.stderr.strip()[:2000])
			)
		return proc.stdout

	# ────────────── 四原语 ──────────────

	def init(self, output: str) -> str:
		"""空文档初始化（init 组合技 = msgs 模板 + save 原语）。

		写最小 legacy `.graphite` 模板（msgs.empty_document_template()）到临时文件 →
		``save`` 原语经编辑器完整管线转换为编辑器形态 `.gdd`。实测验收：
		inspect 读回 ``source=editor``、``nodeCount=0``，且后续 exec --save 的改动
		跨进程持久。返回落盘路径。
		"""
		template = json.dumps(_msgs.empty_document_template(), ensure_ascii=False)
		tmp_path = None
		try:
			with tempfile.NamedTemporaryFile("w", suffix=".graphite", delete=False, encoding="utf-8") as tmp:
				tmp.write(template)
				tmp_path = tmp.name
			result = self.save(tmp_path, output=output)
			if not result.path:
				raise EngineError("init: save produced no parseable path (raw: %r)" % result.raw)
			return result.path
		finally:
			if tmp_path:
				try:
					os.unlink(tmp_path)
				except OSError:
					pass

	def save(self, document: str, output: Optional[str] = None) -> SaveResult:
		"""原语 1：转存 .gdd。参数名以 main.rs 现状为准：``--output``/``-o``。"""
		args = ["save", str(document)]
		if output:
			args += ["--output", str(output)]
		stdout = self._run(args)
		return self._parse_save_stdout(stdout)

	@staticmethod
	def _parse_save_stdout(stdout: str) -> SaveResult:
		# main.rs: "Saved {doc} -> {path} ({bytes} bytes)"——宽松解析，不匹配也不炸。
		result = SaveResult(raw=stdout.strip())
		parts = result.raw.replace("(", " ").replace(")", " ").split()
		if result.raw.startswith("Saved") and "->" in parts and len(parts) >= 6:
			try:
				result.document = parts[1]
				result.path = parts[parts.index("->") + 1]
				result.bytes_written = int(parts[parts.index("bytes") - 1])
			except (ValueError, IndexError):
				pass
		return result

	def inspect(self, document: str, pretty: bool = False) -> InspectResult:
		"""原语 2：读文档结构（PY-2 契约：source/nodeCount/nodes/wires/layers）。"""
		args = ["inspect", str(document)]
		if pretty:
			args.append("--pretty")
		return InspectResult.from_stdout(self._run(args))

	def exec_messages(self, document: str, messages, save_to: Optional[str] = None) -> ExecResult:
		"""原语 3：注入 Message dict 数组执行。

		传递通道：写入临时文件走 ``--file``（exec 终态无 stdin 缺省；避开 ARG_MAX）。
		``--save <out>`` = 批后存盘 .gdd。整批由 Rust 侧自动包事务。
		"""
		args = ["exec", str(document)]
		payload = json.dumps(list(messages), ensure_ascii=False)
		tmp_path = None
		try:
			with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as tmp:
				tmp.write(payload)
				tmp_path = tmp.name
			args += ["--file", tmp_path]
			if save_to:
				args += ["--save", str(save_to)]
			stdout = self._run(args)
		finally:
			if tmp_path:
				try:
					os.unlink(tmp_path)
				except OSError:
					pass
		return ExecResult.from_stdout(stdout)

	def export(self, document: str, output: str, width=None, height=None, scale=None, transparent=None) -> ExportResult:
		"""原语 4：编辑器链导出（.svg/.png 按扩展名；--width/--height 走两步校正）。

		⚠ 文档需含 artboard（msgs.new_artboard），否则编辑器弹 Error → EngineError。
		"""
		args = ["export", str(document), "-o", str(output)]
		if width is not None:
			args += ["--width", str(width)]
		if height is not None:
			args += ["--height", str(height)]
		if scale is not None:
			args += ["--scale", str(scale)]
		if transparent:
			args.append("--transparent")
		return ExportResult.from_stdout(self._run(args))
