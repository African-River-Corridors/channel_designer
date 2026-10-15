"""The frozen parity copy of the Python code can never be imported by the shipped package's name.

Spec 0001: the Python code that writes the Reference Results lives under a different import name,
so a test can never import the wrong one by accident. Risk stopped: the parity oracle and the
shipped package silently become the same code (or the shipped name loads the frozen copy).
"""
import ast
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FROZEN_ROOT = ROOT / "parity" / "frozen_python"
FROZEN_PKG = "channel_designer_frozen"
SHIPPED_PKG = "channel_designer"


def _imported_names(path: Path) -> list[str]:
    names = []
    for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
        if isinstance(node, ast.Import):
            names += [a.name for a in node.names]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            names.append(node.module)
    return names


def test_frozen_root_holds_only_the_frozen_package():
    entries = sorted(p.name for p in FROZEN_ROOT.iterdir() if p.name != "__pycache__")
    assert entries == [FROZEN_PKG]


def test_frozen_code_never_names_the_shipped_package():
    bad = []
    for path in (FROZEN_ROOT / FROZEN_PKG).rglob("*.py"):
        for name in _imported_names(path):
            if name.split(".")[0] == SHIPPED_PKG:
                bad.append(f"{path.relative_to(ROOT)}: {name}")
    assert not bad, bad


def test_frozen_code_never_imports_the_shipped_name_by_string():
    """`import_module("channel_designer...")` and `__import__(...)` count too. A plain label such as
    a drawing's provenance text is not an import."""
    bad = []
    for path in (FROZEN_ROOT / FROZEN_PKG).rglob("*.py"):
        for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
            if not isinstance(node, ast.Call):
                continue
            name = getattr(node.func, "attr", None) or getattr(node.func, "id", "")
            if name in {"import_module", "__import__"}:
                for a in node.args:
                    if isinstance(a, ast.Constant) and str(a.value).split(".")[0] == SHIPPED_PKG:
                        bad.append(f"{path.relative_to(ROOT)}:{node.lineno}")
    assert not bad, bad


def test_shipped_name_resolves_to_the_shipped_package_even_with_frozen_root_on_path():
    code = (
        "import sys; sys.path.insert(0, sys.argv[1]);"
        "import channel_designer, channel_designer.volumes.cut_model as c;"
        "print(c.__file__)"
    )
    out = subprocess.run([sys.executable, "-c", code, str(FROZEN_ROOT)],
                         capture_output=True, text=True, check=True).stdout.strip()
    assert "frozen" not in out
    assert Path(out).parts[-3:] == (SHIPPED_PKG, "volumes", "cut_model.py")


def test_frozen_package_imports_under_its_own_name_and_loads_no_shipped_module():
    code = (
        "import sys; sys.path.insert(0, sys.argv[1]);"
        "from channel_designer_frozen.volumes.cut_model import cut_model_a_face;"
        "import channel_designer_frozen.alignment.route, channel_designer_frozen.alignment.rolling_circle;"
        "print(cut_model_a_face(0.0, False, 5.0, 1.0, 4.0, 3.0));"
        "print(sorted(m for m in sys.modules if m == 'channel_designer' or m.startswith('channel_designer.')))"
    )
    out = subprocess.run([sys.executable, "-c", code, str(FROZEN_ROOT)],
                         capture_output=True, text=True, check=True).stdout.split("\n")
    assert out[0] == "1.0"
    assert out[1] == "[]"
