"""The parity driver writes Reference Results in the spec 0001 text form.

Floats are shortest round-trip text; NaN and infinity are tagged strings; a ValueError is an
`error` result. Risk stopped: the oracle file loses precision or cannot carry non-finite values.
"""
import importlib.util
import json
import math
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "parity" / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


driver = _load("driver")


def test_non_finite_floats_become_tagged_strings():
    out = json.loads(driver.dumps(driver.to_json([math.nan, math.inf, -math.inf, 1.5])))
    assert out == ["float:nan", "float:inf", "float:-inf", 1.5]


def test_tagged_strings_in_a_case_input_become_floats():
    out = driver.from_json({"a": ["float:inf", "float:-inf", "float:nan", "text", 2]})
    assert out["a"][0] == math.inf and out["a"][1] == -math.inf
    assert math.isnan(out["a"][2]) and out["a"][3:] == ["text", 2]


def test_floats_are_written_as_shortest_round_trip_text():
    text = driver.dumps({"x": 0.1 + 0.2, "y": 1e-9, "z": 450.0})
    assert '"x": 0.30000000000000004' in text
    assert '"y": 1e-09' in text
    assert '"z": 450.0' in text
    assert json.loads(text)["x"] == 0.1 + 0.2


def test_numpy_values_are_plain_json():
    import numpy as np
    out = driver.to_json({"a": np.array([1.0, np.inf]), "b": np.bool_(True), "c": np.int64(3)})
    assert out == {"a": [1.0, "float:inf"], "b": True, "c": 3}
    assert type(out["b"]) is bool


def test_value_error_is_recorded_with_its_message():
    args = dict(zs=[1.0], xs=[0.0], sidewhich=[1], hb=1.0, grade=0.0, wl=1.0, depth=1.0,
                model="v21", wet=[False])
    out = driver.run_case("cut_model", {"fn": "section_area", "args": args})
    assert out["error"]["type"] == "ValueError"
    assert out["error"]["message"] == (
        "unknown cut model 'v21'. Known: 'cut_model_a' (A) and 'cut_model_b' (B).")


def test_driver_runs_the_frozen_copy_not_the_shipped_package():
    mod = driver._module("volumes.cut_model")
    assert mod.__name__ == "channel_designer_frozen.volumes.cut_model"
    assert "frozen_python" in Path(mod.__file__).parts
