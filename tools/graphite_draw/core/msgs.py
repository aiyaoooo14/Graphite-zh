"""Message JSON 构造层（CLI 方案 §〇.2：serde 形状常量集中处）。

────────────────────────────────────────────────────────────────────────────
【形状契约 · PY-2 实测版】（2026-10-08 联调确认，基准：cli/docs/message-shapes.md
+ 本目录 lab 实验；此前 PY-1 的"推断"项已全部实测校正）
────────────────────────────────────────────────────────────────────────────
editor 的 Message 枚举经 serde **externally tagged** 逐层嵌套，顶层注入
`graphite-cli exec` 的 JSON 数组形状::

    {"Portfolio": {"Document": {"NodeGraph": {"SetInputValue": {…字段…}}}}}
    {"Portfolio": {"Document": "AddTransaction"}}          ← unit 变体 = 裸字符串
    {"Batched":   {"messages": [ …msg, … ]}}
    {"Preferences": {"SaveAsGdd": {"enabled": true}}}      ← ⚠ 只有一层，不走 Portfolio！

⚠ **变体名层不可省**（PY-2 exec 真跑实测）：NodeGraphMessage/GraphOperationMessage
的每条消息都要包一层变体名（`{"CreateWire": {…}}`、`{"NewCustomLayer": {…}}`…），
漏掉即 serde 报 `unknown variant 'node_id'`（PY-1 构造层曾整层缺失，联调才暴露——
这正是"形状必须真进程验证"的教训）。本文件所有构造函数已带变体名。

各叶子类型的 serde 形状（✅ = serde_json 实测确认）：

| Rust 类型              | JSON 形状                                    | 状态 |
|------------------------|----------------------------------------------|------|
| NodeId(newtype u64)    | 裸数字 `5`                                   | ✅ |
| LayerNodeIdentifier    | node_id+1 的裸数字；ROOT_PARENT=1            | ✅ message-shapes.md §2 |
| DAffine2               | 6 元素扁平数组（列主序 `[c0x,c0y,c1x,c1y,c2x,c2y]`） | ✅ message-shapes.md §2 |
| TaggedValue            | externally tagged：`{"F64":42.5}`/`{"Bool":true}`/`{"String":"hi"}`；unit 变体 `"None"`=裸字符串 | ✅ §2 |
| DefinitionIdentifier   | `{"type":"Network","data":"Rectangle"}`（data=**剥前缀裸名**）；ProtoNode 的 data=`{"name":"路径"}` | ✅ §2（PY-1 曾错带 NETWORK: 前缀） |
| InputConnector         | `{"node":{"nodeId":5,"inputIndex":0}}` / `{"export":0}`（camelCase 特例） | ✅ §1 |
| OutputConnector        | `{"node":{"nodeId":5,"outputIndex":0}}` / `{"import":0}` | ✅ §1 |
| 消息变体字段名         | **snake_case**（serde 默认）                 | ✅ §1 |
| TransformIn::Local     | `"Local"`（unit 变体裸字符串）；Scope 变体 `{"Scope":{"scope":[6f]}}` | ✅ 本轮 exec 实测 + §1 |
| CreateNodeFromContextMenu.xy | `[x, y]` / `null`                      | ✅ 本轮 exec 实测 |
| NewCustomLayer / TransformSet / SetInputValue / CreateWire / DisconnectInput / DeleteNodes / SelectedNodesSet / NewArtboard | 见下方构造函数 | ✅ 本轮 exec 注入 + inspect 读回实测 |

────────────────────────────────────────────────────────────────────────────
【事务语义 · PY-2 实验结论（重要，防回归）】
────────────────────────────────────────────────────────────────────────────
`graphite-cli exec` **整批自动包 `StartTransaction → 消息 → EndTransaction`**
（exec.rs::execute），registry 快照在 CommitTransaction 时落盘。实验（cli 终态实测）：

- 裸操作数组 + `--save` → 重开值持久 ✅（exec 事务覆盖）
- 裸操作 + 显式 `RunDocumentGraph` 收尾 → 持久 ✅（RunGraph 无害且触发重渲染）
- 叠加 Python 层 `AddTransaction` 封装 → exec 报 error_count=0 但**重开值回滚** ❌
  （AddTransaction 展开的 Start/Commit 对压队头后立即提交，事务状态置 Finished，
  外层 EndTransaction 不再提交 → 只改内存不落 registry 快照 = 双写漂移）
- `CreateNodeFromContextMenu{add_transaction:true}`（消息内嵌事务）同样静默丢失 ❌；
  `add_transaction:false` 则持久 ✅

因此本层约定：**Python 皮不构造 AddTransaction/StartTransaction**；所有原子操作走
"裸操作数组 + RunDocumentGraph 收尾"；带内嵌事务开关的高层消息一律关掉
（`add_transaction=False`）。`transaction()` 保留函数名但形状已改为无事务壳。

原则：**任何形状改动只改本文件**，session/engine/CLI 层不感知 JSON 细节。
"""

# ────────────────────────── 顶层嵌套常量 ──────────────────────────

PORTFOLIO = "Portfolio"        # Message::Portfolio(PortfolioMessage)
DOCUMENT = "Document"          # PortfolioMessage::Document(DocumentMessage)
NODE_GRAPH = "NodeGraph"       # DocumentMessage::NodeGraph(NodeGraphMessage)
GRAPH_OPERATION = "GraphOperation"  # DocumentMessage::GraphOperation(GraphOperationMessage)
PREFERENCES = "Preferences"    # Message::Preferences(PreferencesMessage)——顶层直连！
BATCHED = "Batched"            # Message::Batched{messages}

# LayerNodeIdentifier::ROOT_PARENT 在 JSON 里的值（= NonZeroU64(NodeId(0)+1)）。
ROOT_PARENT_JSON = 1


# ────────────────────────── 顶层包装 ──────────────────────────

def node_graph(inner) -> dict:
	"""包成 `{"Portfolio":{"Document":{"NodeGraph":inner}}}`。"""
	return {PORTFOLIO: {DOCUMENT: {NODE_GRAPH: inner}}}


def graph_operation(inner) -> dict:
	"""包成 `{"Portfolio":{"Document":{"GraphOperation":inner}}}`。"""
	return {PORTFOLIO: {DOCUMENT: {GRAPH_OPERATION: inner}}}


def document_message(inner) -> dict:
	"""包成 `{"Portfolio":{"Document":inner}}`；inner 传字符串 = unit 变体。"""
	return {PORTFOLIO: {DOCUMENT: inner}}


def preferences(inner) -> dict:
	"""包成 `{"Preferences":inner}`。

	⚠ 实测（message-shapes.md §1）：PreferencesMessage 从 Message 顶层直达，
	**没有** Portfolio 层——PY-1 曾错包一层。
	"""
	return {PREFERENCES: inner}


def batched(messages) -> dict:
	"""`{"Batched":{"messages":[…]}}` —— editor 的一次性多消息注入形状。"""
	return {BATCHED: {"messages": list(messages)}}


# ────────────────────────── 叶子类型 ──────────────────────────

def layer_ref(node_id) -> int:
	"""NodeId → LayerNodeIdentifier 的 JSON 数字：`NonZeroU64(node_id + 1)`。

	坑：JSON 里看到的 layer 数字一律比 NodeId 大 1；ROOT_PARENT 是 `1` 而不是 `0`。
	"""
	return node_id + 1


def input_connector_node(node_id, input_index) -> dict:
	"""`{"node":{"nodeId":N,"inputIndex":I}}`（camelCase 特例）。"""
	return {"node": {"nodeId": node_id, "inputIndex": input_index}}


def input_connector_export(index) -> dict:
	"""`{"export":I}`。"""
	return {"export": index}


def output_connector_node(node_id, output_index) -> dict:
	"""`{"node":{"nodeId":N,"outputIndex":I}}`。"""
	return {"node": {"nodeId": node_id, "outputIndex": output_index}}


def output_connector_import(index) -> dict:
	"""`{"import":I}`。"""
	return {"import": index}


def daffine2(cols) -> list:
	"""6 元素列主序数组 `[c0x,c0y, c1x,c1y, c2x,c2y]`（glam from_cols_array 顺序）。

	cols=单位阵 → `[1,0, 0,1, 0,0]`；平移 (tx,ty) → `[1,0, 0,1, tx,ty]`。
	"""
	if len(cols) != 6:
		raise ValueError("DAffine2 needs exactly 6 column-major floats, got %d" % len(cols))
	return [float(c) for c in cols]


def identity_matrix() -> list:
	"""DAffine2 单位阵。"""
	return daffine2([1, 0, 0, 1, 0, 0])


def translation_matrix(tx, ty) -> list:
	"""DAffine2 纯平移。"""
	return daffine2([1, 0, 0, 1, tx, ty])


def definition_identifier_network(name) -> dict:
	"""Network 节点定义：`{"type":"Network","data":name}`。

	⚠ 实测（message-shapes.md §2）：data 是**裸名**（如 "Rectangle"/"Merge"），
	不带 "NETWORK:" 前缀——PY-1 曾原样透传前缀。
	"""
	return {"type": "Network", "data": name}


def definition_identifier_proto(name) -> dict:
	"""ProtoNode 节点定义：`{"type":"ProtoNode","data":{"name":…}}`。

	name = proto 路径（如 `"vector_nodes::generator_nodes::RectangleNode"`）。
	"""
	return {"type": "ProtoNode", "data": {"name": name}}


def definition_identifier(serialized) -> dict:
	"""按前缀路由 serialized 定义名（剥前缀，实测形状）：

	- `"NETWORK:Merge"`  → `{"type":"Network","data":"Merge"}`
	- `"PROTONODE:vector_nodes::…"` → `{"type":"ProtoNode","data":{"name":"vector_nodes::…"}}`
	- 裸名 → ValueError（显示名解析是上层职责，msgs 只认 serialized 形式）。
	"""
	if serialized.startswith("NETWORK:"):
		return definition_identifier_network(serialized[len("NETWORK:"):])
	if serialized.startswith("PROTONODE:"):
		return definition_identifier_proto(serialized[len("PROTONODE:"):])
	raise ValueError(
		"node type must be a serialized identifier (NETWORK:… / PROTONODE:…), got %r" % (serialized,)
	)


# ────────────────────────── TaggedValue 便捷构造（实测拼写） ──────────────────────────

def tagged(variant, payload) -> dict:
	"""显式 TaggedValue：`tagged("F64", 200.0)` → `{"F64":200.0}`。"""
	return {variant: payload}


def tagged_none() -> str:
	"""unit 变体 `None` 在 externally tagged 下是**裸字符串** `"None"`。"""
	return "None"


def tagged_f64(value) -> dict:
	return tagged("F64", float(value))


def tagged_bool(value) -> dict:
	return tagged("Bool", bool(value))


def tagged_string(value) -> dict:
	return tagged("String", str(value))


def tagged_box_corners(values) -> dict:
	"""`{"BoxCorners":[…]}`（Rectangle 圆角实测变体，PY-2 inspect 读回确认）。"""
	return tagged("BoxCorners", [float(v) for v in values])


# ────────────────────────── 批次组装 ──────────────────────────

def run_document_graph() -> dict:
	"""`NodeGraphMessage::RunDocumentGraph`（unit 变体）——批次收尾，触发重编译/渲染。"""
	return node_graph("RunDocumentGraph")


def transaction(operations) -> list:
	"""批次惯用法（PY-2 实验结论后的形态）：**操作们 + RunDocumentGraph**，裸数组。

	⚠ 历史注记：PY-1 形状是 `AddTransaction → 操作 → RunGraph` 包 Batched；PY-2 实测
	该封装与 exec 的自动事务（StartTransaction→…→EndTransaction）叠加会导致改动
	静默回滚（见模块头【事务语义】），故**去掉 AddTransaction**、不再包 Batched
	（exec 原语吃裸数组），保留 RunGraph 收尾。返回 Message 数组（非单条）。
	"""
	messages = list(operations)
	messages.append(run_document_graph())
	return messages


# ────────────────────────── 八件套原子操作 ──────────────────────────
# 每个函数返回**一条** Message dict；批次组装统一走 transaction()。

def create_layer(node_id, parent_node_id=None, insert_index=0, nodes=None) -> dict:
	"""`GraphOperationMessage::NewCustomLayer{id, nodes, parent, insert_index}`。

	parent 传 None = ROOT_PARENT。无内嵌事务（PY-2 实测持久 ✅）。
	node_id 由调用方（NodeIdAllocator）预先分配。
	"""
	return graph_operation({"NewCustomLayer": {
		"id": node_id,
		"nodes": list(nodes or []),
		"parent": layer_ref(parent_node_id) if parent_node_id is not None else ROOT_PARENT_JSON,
		"insert_index": insert_index,
	}})


def add_node_via_context_menu(node_id, node_type, position=None, add_transaction=False) -> dict:
	"""`NodeGraphMessage::CreateNodeFromContextMenu{node_id, node_type, xy, add_transaction}`。

	⚠ PY-2 实测：`add_transaction=True`（消息内嵌事务）在 exec 自动事务内会静默丢失
	改动（保存重开节点消失）；必须 **False**，交由 exec 的整批事务负责持久化。
	position=None → `xy: null`（实测接受）。node_type 用 definition_identifier() 的产物。
	"""
	return node_graph({"CreateNodeFromContextMenu": {
		"node_id": node_id,
		"node_type": node_type,
		"xy": [position[0], position[1]] if position is not None else None,
		"add_transaction": add_transaction,
	}})


def add_nodes(nodes, new_ids=None) -> dict:
	"""`NodeGraphMessage::AddNodes{nodes: Vec<(NodeId, NodeTemplate)>, new_ids: HashMap}`。

	低层形状保留（任务书映射参考）。nodes 是 `[(node_id, template_dict), …]`；
	NodeTemplate 的 JSON = NodeTemplateParts：`{"document_node":{…},"persistent_node_metadata":{…}}`
	（template.rs:14 `#[serde(from/into = "NodeTemplateParts")]`）——模板构造极深，PY-2
	不提供构造器，只做形状透传；建点实际走 add_node_via_context_menu 高层消息。
	"""
	return node_graph({"AddNodes": {
		"nodes": [[node_id, template] for node_id, template in nodes],
		"new_ids": dict(new_ids or {}),
	}})


def create_wire(from_node, from_output, to_node, to_input) -> dict:
	"""`NodeGraphMessage::CreateWire{output_connector, input_connector}`（实测持久 ✅）。"""
	return node_graph({"CreateWire": {
		"output_connector": output_connector_node(from_node, from_output),
		"input_connector": input_connector_node(to_node, to_input),
	}})


def disconnect_input(to_node, to_input) -> dict:
	"""`NodeGraphMessage::DisconnectInput{input_connector}`。"""
	return node_graph({"DisconnectInput": {"input_connector": input_connector_node(to_node, to_input)}})


def set_input_value(node_id, input_index, value) -> dict:
	"""`NodeGraphMessage::SetInputValue{node_id, input_index, value: TaggedValue}`（实测持久 ✅）。

	value 用 tagged*() 构造，或已形状正确的 dict 透传。**注意变体必须与现值一致**
	（message-shapes.md §2：先 inspect 看变体再设）。
	"""
	return node_graph({"SetInputValue": {
		"node_id": node_id,
		"input_index": input_index,
		"value": value,
	}})


def transform_set(layer_node_id, matrix, transform_in="Local", skip_rerender=False) -> dict:
	"""`GraphOperationMessage::TransformSet{layer, transform, transform_in, skip_rerender}`。

	layer 传 **NodeId**（内部 +1，实测接受）；matrix 是 daffine2() 的 6 元素数组；
	`transform_in="Local"`（unit 变体裸字符串，实测 ✅；Scope 变体 `{"Scope":{"scope":[6f]}}`）。
	"""
	return graph_operation({"TransformSet": {
		"layer": layer_ref(layer_node_id),
		"transform": matrix,
		"transform_in": transform_in,
		"skip_rerender": skip_rerender,
	}})


def delete_nodes(node_ids, delete_children=True) -> dict:
	"""`NodeGraphMessage::DeleteNodes{node_ids, delete_children}`。

	⚠ 不要用 `DocumentMessage::DeleteNode`：它内嵌 Start/End 事务
	（document_message_handler.rs:389-398），在 exec 自动事务内同 CreateNodeFromContextMenu
	的事务嵌套风险。坑#4：失败只在编辑器日志里，commit 后靠 inspect 验证。
	"""
	return node_graph({"DeleteNodes": {"node_ids": list(node_ids), "delete_children": delete_children}})


def selected_nodes_set(node_ids) -> dict:
	"""`NodeGraphMessage::SelectedNodesSet{nodes}`（坑#9：选区是全局状态，显式设置）。"""
	return node_graph({"SelectedNodesSet": {"nodes": list(node_ids)}})


def new_artboard(node_id, location=(0.0, 0.0), dimensions=(400.0, 300.0),
                 background=(1.0, 1.0, 1.0, 1.0), clip=True) -> dict:
	"""`GraphOperationMessage::NewArtboard{id, location, dimensions, background, clip}`。

	⚠ PY-2 实测：**export 需要文档里有 artboard**，否则编辑器渲染链弹 Error 对话框
	（ExportBounds::AllArtwork 无 artboard 时失败）。Color 形状
	`{"red","green","blue","alpha","linear"}`（demo-artwork 实测样例）。
	"""
	r, g, b, a = background
	return graph_operation({"NewArtboard": {
		"id": node_id,
		"location": [float(location[0]), float(location[1])],
		"dimensions": [float(dimensions[0]), float(dimensions[1])],
		"background": {"red": float(r), "green": float(g), "blue": float(b), "alpha": float(a), "linear": True},
		"clip": clip,
	}})


# ────────────────────────── 文档级杂项 ──────────────────────────

def save_document() -> dict:
	"""`DocumentMessage::SaveDocument`（unit 变体）。"""
	return document_message("SaveDocument")


def save_as_gdd(enabled=True) -> dict:
	"""`PreferencesMessage::SaveAsGdd{enabled}`——一层形状（坑#2：默认 false）。

	注：exec `--save` / save 原语内部已自动设置，一般无需手工注入。
	"""
	return preferences({"SaveAsGdd": {"enabled": enabled}})


# ────────────────────────── 空文档模板（init） ──────────────────────────

def empty_document_template() -> dict:
	"""最小可打开的 legacy `.graphite` 文档（空节点表）。

	PY-2 实测路径：`写此模板 → graphite-cli save → 编辑器形态空 .gdd`
	（inspect 断言 source=editor、nodeCount=0 ✅）。

	形状依据：
	- `DocumentMessageHandler` serde：network_interface/document_ptz/render_mode/
	  overlays_visibility_settings/rulers_visible/snapping_state/graph_view_overlay_open/
	  graph_fade_artwork_percentage 无 default，**必须给**（document_message_handler.rs:73+）；
	  collapsed/resources/properties_panel_collapsed_sections 有 default 可省（显式给出更稳）。
	- `NodeNetworkInterface` 需要 network + network_metadata.persistent_metadata
	  （缺 network_metadata 报 `missing field`，实测）。
	- 空网络的 exports 不能是 `[]`：图扁平化报 "No input type: []"（实测）；
	  编辑器默认文档带一个 `TypeDefault(list!(Artboard))` 值导出
	  （default_document_network_interface，document_message_handler.rs:3777）。
	  TypeDescriptor serde = `{"name":"graphic_types::artboard::Artboard"}`（demo-artwork 实测）。
	- overlays_visibility_settings/snapping_state/navigation_metadata 抄编辑器实测样例
	  （demo-artwork），值不影响空文档语义。
	"""
	return {
		"network_interface": {
			"network": {
				"exports": [{
					"Value": {
						"tagged_value": {"TypeDefault": {
							"List": {"Concrete": {"name": "graphic_types::artboard::Artboard"}},
						}},
						"exposed": True,
					},
				}],
				"nodes": [],
			},
			"network_metadata": {
				"persistent_metadata": {
					"reference": None,
					"node_metadata": [],
					"pinned_node_order": [],
					"previewing": "No",
					"navigation_metadata": {
						"node_graph_ptz": {"pan": [0.0, 0.0], "tilt": 0.0, "zoom": 1.0, "flip": False},
						"node_graph_to_viewport": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
						"node_graph_width": 1980.0,
					},
				}
			},
		},
		"collapsed": [],
		"properties_panel_collapsed_sections": [],
		"document_ptz": {"pan": [0.0, 0.0], "tilt": 0.0, "zoom": 1.0, "flip": False},
		"render_mode": "Normal",
		"overlays_visibility_settings": {
			"all": True, "artboard_name": True, "compass_rose": True,
			"quick_measurement": True, "transform_measurement": True,
		},
		"rulers_visible": True,
		"snapping_state": {
			"snapping_enabled": True, "grid_snapping": False, "artboards": True,
			"tolerance": 8.0,
		},
		"graph_view_overlay_open": False,
		"graph_fade_artwork_percentage": 80.0,
	}
