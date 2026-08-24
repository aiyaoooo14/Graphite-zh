#!/bin/sh
# Graphite-zh 字体安装脚本
# 将随仓库分发的 HarmonyOS Sans SC 字体安装到用户字体目录（无需 root）。
# 幂等：同名字体已存在则跳过；完成后刷新 fontconfig 缓存。
# 许可：HarmonyOS Sans 字体允许免费商用与随应用分发（https://www.harmonyos.com），
#       分发时请保留字体文件内嵌的许可声明。

set -eu

FONT_DIR_SOURCE="$(cd "$(dirname "$0")" && pwd)"
FONT_DIR_TARGET="${HOME}/.local/share/fonts"

mkdir -p "$FONT_DIR_TARGET"

installed=0
skipped=0
for font_file in "$FONT_DIR_SOURCE"/*.ttf; do
	[ -e "$font_file" ] || continue
	target_file="$FONT_DIR_TARGET/$(basename "$font_file")"
	if [ -e "$target_file" ]; then
		echo "跳过（已存在）：$(basename "$font_file")"
		skipped=$((skipped + 1))
	else
		cp "$font_file" "$target_file"
		echo "已安装：$(basename "$font_file")"
		installed=$((installed + 1))
	fi
done

if command -v fc-cache >/dev/null 2>&1; then
	fc-cache -f "$FONT_DIR_TARGET" >/dev/null 2>&1 || true
	echo "fontconfig 缓存已刷新。"
else
	echo "警告：未找到 fc-cache，请手动执行：fc-cache -f $FONT_DIR_TARGET"
fi

echo "完成：新装 $installed 个，跳过 $skipped 个。"
echo "验证：fc-list | grep -i \"HarmonyOS Sans SC\""
