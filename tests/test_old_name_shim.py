"""`import channel_designer` still works for one release, warns, and gives the `owd` objects.

Each case runs in a fresh interpreter: the shim edits `sys.modules` and `sys.meta_path`.
Label: property
"""
import subprocess
import sys
import textwrap


def _run(code: str, *flags: str) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, *flags, "-c", textwrap.dedent(code)],
                          capture_output=True, text=True, check=False)


def test_import_warns_deprecation():
    r = _run("import channel_designer", "-W", "always")
    assert r.returncode == 0, r.stderr
    assert "DeprecationWarning" in r.stderr and "import owd" in r.stderr


def test_warning_can_be_turned_into_an_error():
    r = _run("import channel_designer", "-W", "error::DeprecationWarning")
    assert r.returncode != 0 and "DeprecationWarning" in r.stderr


def test_package_and_submodules_are_the_owd_objects():
    r = _run("""
        import warnings
        warnings.simplefilter("ignore")
        import owd
        import channel_designer
        assert channel_designer is owd
        import channel_designer.rules.rulesets as old_rulesets
        import owd.rules.rulesets as new_rulesets
        assert old_rulesets is new_rulesets
        assert new_rulesets.__spec__.name == "owd.rules.rulesets"
        from channel_designer.rules import channel_shape
        assert channel_shape is owd.rules.channel_shape
        from channel_designer.volumes.sections import section_volumes
        assert section_volumes is owd.volumes.sections.section_volumes
        import channel_designer._core as old_core
        import owd._core as new_core
        assert old_core is new_core
        import channel_designer.alignment as old_alignment
        assert old_alignment is owd.alignment
    """)
    assert r.returncode == 0, r.stderr


def test_new_name_does_not_warn():
    r = _run("import owd, owd.rules", "-W", "error::DeprecationWarning")
    assert r.returncode == 0, r.stderr
