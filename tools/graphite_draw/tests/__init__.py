"""unittest 包引导：把 tools/graphite_draw 放进 sys.path，让测试用 `from core…` 导入。

支持从仓库根直接跑：
    python3 -m unittest discover tools/graphite_draw/tests
"""

import sys
from pathlib import Path

_PACKAGE_ROOT = str(Path(__file__).resolve().parents[1])
if _PACKAGE_ROOT not in sys.path:
	sys.path.insert(0, _PACKAGE_ROOT)
