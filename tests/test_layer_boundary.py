"""Layer rules: calculation code reads no files and pulls no heavy geo stack.

- rules / alignment / volumes (except volumes.raster) import only numpy, shapely,
  vessel_designer and the standard library;
- nothing in src/ hacks sys.path or names a machine-local path.
"""
import ast
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / "src" / "channel_designer"
ALLOWED_THIRD_PARTY = {"numpy", "shapely", "vessel_designer"}
HEAVY = {"rasterio", "geopandas", "fiona", "pyproj", "scipy", "affine", "osgeo", "gdal",
         "pandas", "matplotlib", "technical_drawings_for_agents"}
EXEMPT = {"volumes/raster.py", "drawings/sections.py"}


def _imports(path):
    tree = ast.parse(path.read_text())
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for a in node.names:
                yield a.name.split(".")[0]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            yield node.module.split(".")[0]


def test_calculation_layers_import_no_heavy_geo_stack():
    bad = []
    for py in SRC.rglob("*.py"):
        rel = py.relative_to(SRC).as_posix()
        if rel in EXEMPT:
            continue
        for mod in _imports(py):
            if mod in HEAVY:
                bad.append(f"{rel}: {mod} (heavy)")
            elif (mod not in sys.stdlib_module_names and mod not in ALLOWED_THIRD_PARTY
                  and mod != "channel_designer"):
                bad.append(f"{rel}: {mod} (not an allowed core dependency)")
    assert not bad, bad


def test_no_sys_path_hacks_or_file_io_in_calculation_layers():
    bad = []
    for py in SRC.rglob("*.py"):
        rel = py.relative_to(SRC).as_posix()
        text = py.read_text()
        if "sys.path" in text:
            bad.append(f"{rel}: sys.path")
        if rel.split("/")[0] in ("rules", "alignment", "volumes") and rel not in EXEMPT:
            for needle in ("open(", ".read_text(", ".write_text(", "to_file("):
                if needle in text:
                    bad.append(f"{rel}: {needle}")
    assert not bad, bad
