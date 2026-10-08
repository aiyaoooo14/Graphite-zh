# 安装说明

## 目标结构（实际安装形态）

```
~/.agents/skills/graphite-draw/
├── SKILL.md
├── install.md
├── nodes.json               # node-list 静态清单（286 条，安装时生成）
├── bin/
│   └── graphite-cli         # release 构建 strip 后（约 69MB）
├── graphite_draw/           # 复制自仓库 tools/graphite_draw/（排除 __pycache__）
│   ├── graphite_draw.py
│   ├── core/{__init__,ids,msgs,engine,session}.py
│   ├── examples/end_to_end.py
│   └── tests/
├── sub/*.md
└── recipes/*.md
```

## 安装步骤

```bash
cargo build --release -p graphite-cli
mkdir -p ~/.agents/skills/graphite-draw/bin
cp -r skill/graphite-draw/. ~/.agents/skills/graphite-draw/
rsync -a --exclude='__pycache__' tools/graphite_draw/ ~/.agents/skills/graphite-draw/graphite_draw/
cp target/release/graphite-cli ~/.agents/skills/graphite-draw/bin/
strip ~/.agents/skills/graphite-draw/bin/graphite-cli
~/.agents/skills/graphite-draw/bin/graphite-cli node-list > ~/.agents/skills/graphite-draw/nodes.json
```

## 路径约定（重要）

- `GRAPHITE_DRAW_SKILL` 指向 **graphite_draw 包目录**（即 `~/.agents/skills/graphite-draw/graphite_draw`，仓库内缺省 `tools/graphite_draw`）——配方与示例的 `sys.path` 以它 import `core.*`
- `GRAPHITE_CLI_BIN` 指向 `~/.agents/skills/graphite-draw/bin/graphite-cli`
- `Engine()` 二进制定位顺序：构造参数 → `$GRAPHITE_CLI_BIN` → `target/{debug,release}` → PATH

## 验证安装

```bash
GRAPHITE_CLI_BIN=~/.agents/skills/graphite-draw/bin/graphite-cli \
GRAPHITE_DRAW_SKILL=~/.agents/skills/graphite-draw/graphite_draw \
python3 ~/.agents/skills/graphite-draw/graphite_draw/examples/end_to_end.py
# 期望输出：=== END-TO-END OK === 与 .gdd/.svg/.png 产物路径
```

（更新即重装：重复上述步骤，skill 源随仓库演进）
