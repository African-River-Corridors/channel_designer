"""The CI workflows: the required checks always run, the filtered jobs never block a merge, and every
scheduled workflow reports its failures.

Label: property. No pinned numbers: the tests read .github/workflows.
"""
import re
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
WF = ROOT / ".github" / "workflows"
#: Branch protection requires these check names on main. Renaming one needs a settings change.
REQUIRED = ("test", "release-guard")
#: Jobs in ci.yml that run on a pull request only when their inputs change.
FILTERED = {"app-linux": "app", "app-e2e": "app", "web-parity": "web", "web-app": "web"}


def load(name):
    doc = yaml.safe_load((WF / name).read_text(encoding="utf-8"))
    doc["on"] = doc.pop(True, doc.get("on"))  # YAML 1.1 reads the key `on` as true
    return doc


def test_the_required_jobs_exist_and_always_run_on_every_pull_request():
    ci = load("ci.yml")
    assert "paths" not in (ci["on"]["pull_request"] or {}), "a path filter on ci.yml could skip a required check"
    for name in REQUIRED:
        job = ci["jobs"][name]
        assert "if" not in job, f"{name} must not be conditional"
        needs = job.get("needs", [])
        assert "changes" not in (needs if isinstance(needs, list) else [needs])


def test_a_filtered_job_runs_on_main_and_on_a_pull_request_that_changes_its_inputs():
    ci = load("ci.yml")
    outputs = ci["jobs"]["changes"]["outputs"]
    for name, group in FILTERED.items():
        job = ci["jobs"][name]
        assert job["needs"] == "changes"
        assert job["if"] == f"github.event_name != 'pull_request' || needs.changes.outputs.{group} == 'true'"
        assert group in outputs
    # app-e2e runs on any change under app/ (it caught a real bug in #268).
    flt = ci["jobs"]["changes"]["steps"][-1]["with"]["filters"]
    assert '"app/**"' in flt.split("web:")[0]


def test_the_path_filter_and_codeql_actions_are_pinned_to_a_commit():
    found = 0
    for path in WF.glob("*.yml"):
        for uses in re.findall(r"uses:\s*(\S+)", path.read_text(encoding="utf-8")):
            if uses.startswith(("dorny/paths-filter", "github/codeql-action")):
                found += 1
                assert re.search(r"@[0-9a-f]{40}$", uses), f"{path.name}: {uses} is not pinned to a commit"
    assert found >= 3


def test_every_scheduled_workflow_reports_a_failed_run():
    watched = set(load("scheduled-failure.yml")["on"]["workflow_run"]["workflows"])
    scheduled = {load(p.name)["name"] for p in WF.glob("*.yml") if "schedule" in (load(p.name)["on"] or {})}
    assert scheduled and scheduled <= watched, f"not reported: {scheduled - watched}"


def test_codeql_does_not_run_on_pull_requests():
    assert "pull_request" not in load("codeql.yml")["on"]
