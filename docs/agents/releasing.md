# Releasing

How a version is cut, so main never has to stop merging.

1. **Cut a release branch** from the commit you want to ship: `git switch -c release/0.N.x <commit>`.
   Main keeps merging; nothing needs to be held.
2. **Prepare the release** on that branch: bump the versions (`Cargo.toml`, `Cargo.lock` for the
   workspace crates, `pyproject.toml`, `src/owd/__init__.py`, `app/package.json`,
   `app/package-lock.json`, `app/src-tauri/tauri.conf.json`, the `engine_version` in
   `examples/synthetic-river.owd`), then run `python tools/changelog.py release 0.N.M`. Open a PR
   into the release branch.
3. **Tag** the merged release commit (`git tag -a v0.N.M <commit> -m "Release 0.N.M"`) and push the
   tag by hand. The maintainer approves every tag.
4. **The tag runs `release.yml`.** It builds the wheels and the sdist and attaches them to a GitHub
   release. PyPI publishing is off before 0.4. From 0.4, the `pypi` environment needs the
   maintainer's approval, and `tools/pypi_ready.py` fails the release check if a dependency is
   given by URL, which PyPI refuses.
5. **Bring the changelog back** to main with a PR that merges the release branch's `CHANGELOG.md`.
   This removes the fragments the release consumed.

A fix for the released version goes onto the release branch and is cherry-picked to main, or the other
way round.
