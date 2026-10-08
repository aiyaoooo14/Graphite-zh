# `exec --json` 消息 serde 形状实测记录

> 交付物（CLI 接口方案 L2-4'）。以下形状全部为 **serde_json 实测输出**（非文档推断），
> 由 `cli/tests/primitives.rs::measured_message_serde_shapes` 打印（`SHAPE` 行）并断言
> round-trip 成立。editor 版本基准：本仓库 2026-10-08 工作区状态。
>
> 顶层规则：`Message` 枚举整体 derive serde，**externally tagged 层层嵌套**
> （`Message → Portfolio → Document → NodeGraph → 具体消息`）。

## 1. 常用消息的完整 JSON 形状

### NodeGraphMessage::SetInputValue（改参数）

```json
{"Portfolio":{"Document":{"NodeGraph":{"SetInputValue":{"node_id":333,"input_index":2,"value":{"F64":42.5}}}}}}
```

- `node_id`：u64 数字（NodeId 的内层值，**不是**对象）。
- `value`：`TaggedValue`（见 §2）。
- 若该节点连着输出，编辑器会自动 `RunDocumentGraph`。

### NodeGraphMessage::SelectedNodesSet（设置选区）

```json
{"Portfolio":{"Document":{"NodeGraph":{"SelectedNodesSet":{"nodes":[1,2]}}}}}
```

`nodes`：NodeId 数字数组。选区是全局状态，写操作前应显式设置（CLI 方案 §四.9）。

### PreferencesMessage::SaveAsGdd

```json
{"Preferences":{"SaveAsGdd":{"enabled":true}}}
```

注意：层级只有一层 `Preferences`（不走 `Portfolio`）。

### NodeGraphMessage::CreateWire（连线，参考形状）

```json
{"Portfolio":{"Document":{"NodeGraph":{"CreateWire":{"output_connector":{"node":{"nodeId":111,"outputIndex":0}},"input_connector":{"node":{"nodeId":222,"inputIndex":1}}}}}}}
```

**注意大小写分界**：`InputConnector`/`OutputConnector` 内部字段是 camelCase（`nodeId`/`outputIndex`），
而消息顶层字段（`node_id`/`input_index`）是 snake_case。混用会反序列化失败。

### GraphOperationMessage::TransformSet（层变换，参考形状）

```json
{"Portfolio":{"Document":{"GraphOperation":{"TransformSet":{"layer":244837814094591,"transform":[2.0,0.0,10.0,0.0,3.0,20.0],"transform_in":{"Scope":{"scope":[1.0,0.0,0.0,1.0,0.0,0.0]}},"skip_rerender":false}}}}}
```

## 2. 值类型形状

| 类型 | JSON |
|---|---|
| `DAffine2` | `[2.0,0.0,10.0,0.0,3.0,20.0]`（6 元素列主序数组，**不是**对象） |
| `TaggedValue::Bool` | `{"Bool":true}` |
| `TaggedValue::F32` | `{"F32":1.25}` |
| `TaggedValue::F64` | `{"F64":42.5}` |
| `TaggedValue::U32` | `{"U32":7}` |
| `TaggedValue::String` | `{"String":"hi"}` |
| `TaggedValue::None` | `"None"`（unit 变体 = 裸字符串） |
| `TaggedValue::LegacyOptionalDAffine2(Some)` | `{"LegacyOptionalDAffine2":[1.0,0.0,0.0,1.0,0.0,0.0]}` |
| `TaggedValue::LegacyOptionalDAffine2(None)` | `{"LegacyOptionalDAffine2":null}` |
| `DefinitionIdentifier::Network` | `{"type":"Network","data":"Rectangle"}`（**内部 tagged**：`tag`/`data` 键） |
| `DefinitionIdentifier::ProtoNode` | `{"type":"ProtoNode","data":{"name":"graphene_core::ops::identity::IdentityNode"}}` |
| `LayerNodeIdentifier::ROOT_PARENT` | `1`（裸数字 = NodeId.0 + 1；NodeId(7) → `8`） |

**注**：`TaggedValue` 无名为 `Optional` 的变体；"可选"语义由
`LegacyOptionalDAffine2` / `OptionalColor`(alias → `Color`) 等承担（alias 见
`graph-craft/src/document/value.rs`）。设置参数前先用 `inspect` 看该输入当前变体，
新值必须与旧值**同变体**。

## 3. JSON 无法表达的消息（exec 明确不支持，报清晰错误，不 panic）

- **`UpdateTypes` 类**：payload 含 `serde(skip)` 字段，无法从 JSON 构造。
  实测当前 editor 中 `UpdateTypes` 不是 `FrontendMessage` 变体，注入形如
  `[{"Frontend":{"UpdateTypes":…}}]` 得到：
  `unknown variant `UpdateTypes``（serde 报错，exec 包装为清晰错误信息）。
- **ByteBuf 类消息**（如 `TriggerSaveDocument.content`、图像数据）：JSON 下是数字数组，
  只出不进（编辑器→前端方向可序列化；前端→编辑器方向不接受）。此类消息不进 exec 文档。

## 4. exec 输出契约（stdout 单行 JSON）

```json
{"events":[{"kind":"save_document","name":"…","path":null,"bytes":561}],
 "error_count":0,
 "saved_path":"/abs/path/out.gdd"}
```

`events` 元素的 `kind`：`save_document` / `save_file` / `export_image` / `dialog` /
`suppressed` / `error`。`error_count` = 派发失败数 + 对话框数；非零时命令退出码非零。
`saved_path` 仅在 `--save` 时出现。

## 5. 已知文档形态陷阱（实测）

- **编辑器保存的节点表是数组形态** `[[NodeId, node], …]`（编辑器重序列化时
  HashMap→数组），手写文档是对象形态。`inspect` 两种都认。
- **headless 下 `UpdateNodeGraphNodes` 恒不发出**（依赖 `graph_view_overlay_open`）。
  读结构必须用 `inspect`（直读文档状态），不要等 FrontendMessage。
- **exec 批次必须包事务（CLI 已自动处理）**：图变更消息（如 `SetInputValue`）只改内存
  network_interface；graph-storage registry 的快照在 `CommitTransaction` 时才落
  （`document_message_handler.rs` 的 `commit_storage_snapshot`）。实测裸批次 + 保存会产出
  **双写漂移**的 `.gdd`（legacy blob = 新值、registry = 旧值），重开时编辑器优先从
  registry 加载 → 改动"回滚"。`exec` 因此把整批包进
  `StartTransaction → 消息 → EndTransaction`（只读批次走 Started→Cancel，无副作用）。
  Python 皮若绕开 `exec` 自行拼消息，也必须保持同一惯用法（= 任务书
  `AddTransaction → 操作 → RunDocumentGraph` 惯用法）。
- **compiler-stack `.gdd`**（`graphene-cli new` 产物，无内嵌编辑器文档）不能被编辑器
  打开——尝试打开会在图扁平化时 panic（`graph-craft/src/document.rs:980`，
  `Import index … should always exist`，实测）。CLI 三原语在触编辑器前先探测
  （`inspect::probe_gdd_document`）：`inspect` 走 registry 读路径输出 `"source":"registry"`，
  `exec`/`export` 给清晰错误。
- 编辑器加载顺序：先试 registry→runtime，失败才回退 legacy blob。因此 registry 与
  legacy blob 不一致时以 registry 为准（见上条事务坑）。
