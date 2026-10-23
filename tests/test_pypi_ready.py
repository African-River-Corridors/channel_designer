"""The PyPI check can fail: a direct dependency fails from 0.4 and is only noted before it."""
import importlib.util
from pathlib import Path

CHECK = Path(__file__).resolve().parents[1] / "tools" / "pypi_ready.py"
spec = importlib.util.spec_from_file_location("pypi_ready", CHECK)
pr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pr)

GIT_DEP = '"vessel_designer @ git+https://github.com/example/repo@v1"'


def _pyproject(tmp_path, version, deps):
    p = tmp_path / "pyproject.toml"
    p.write_text(f'[project]\nname = "x"\nversion = "{version}"\ndependencies = [{deps}]\n')
    return p


def test_git_dependency_fails_from_0_4(tmp_path):
    ok, direct = pr.check(_pyproject(tmp_path, "0.4.0", GIT_DEP))
    assert not ok
    assert direct == ["vessel_designer @ git+https://github.com/example/repo@v1"]


def test_git_dependency_is_only_noted_before_0_4(tmp_path):
    ok, direct = pr.check(_pyproject(tmp_path, "0.3.1", GIT_DEP))
    assert ok
    assert len(direct) == 1


def test_pypi_dependencies_pass(tmp_path):
    ok, direct = pr.check(_pyproject(tmp_path, "1.2.0", '"numpy>=1.26", "vessel-designer>=0.2.5,<0.3"'))
    assert ok
    assert direct == []


def test_the_repo_passes():
    assert pr.main(["pypi_ready.py"]) == 0
