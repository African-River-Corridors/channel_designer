# Spec 0006: Rename to Open Waterway Designer

Status: **approved by the maintainer, 2026-10-21.** The name decision sits in the maintainer's own
records; this spec holds what the builders need.

## Problem statement

The tool is called `channel_designer`. Two problems follow.

- "Open channel" is the hydraulics term for canals, drains and culverts. A name built on "channel"
  tells an engineer the wrong job. "Inland waterway" is the term PIANC WG 141 uses.
- The repo, package, commands and app all use the old name. The sister project (open-vessel-concept-drawings)
  shows what happens when the repo name and the product name disagree: people cannot tell which is which.

The maintainer chose the name **Open Waterway Designer (OWD)** and chose to rename the code to match,
first, before any brand work, so that later work starts on the final names.

## Solution

Rename everything, in small steps, so that `main` stays green after each one (expand, migrate, contract).
For one release the old names still work.

| Thing | Today | New |
|---|---|---|
| GitHub repo | `channel_designer` | `open-waterway-designer` |
| Python import | `channel_designer` | `owd` |
| Python distribution | `channel_designer` (not on PyPI yet) | `open-waterway-designer` |
| Rust crates | `cd-core`, `cd-format`, `cd-cli`, `cd-py`, `cd-app` | `owd-core`, `owd-format`, `owd-cli`, `owd-py`, `owd-app` |
| CLI | `cdesign` | `owd` |
| File extension | `.cdesign` | `.owd` |
| JSON Schema file | `schema/cdesign.schema.json` | `schema/owd.schema.json` |
| App name | Channel Designer | Open Waterway Designer |
| App bundle identifier | `org.africanrivercorridors.channeldesigner` | `org.africanrivercorridors.openwaterwaydesigner` |

For one release (the release that ships this rename), the old names keep working:

- `import channel_designer` works, gives a `DeprecationWarning`, and gives the same objects as `owd`,
  including every submodule (`channel_designer.rules` is `owd.rules`).
- `cdesign` is an alias of `owd`. It prints a one-line deprecation note on stderr.
- A `.cdesign` file opens in the CLI and the app. Saving writes `.owd`. The document contents do not
  change: `format_version` stays 2.

The crates are not published, so they get no shims. The GitHub repo rename is the maintainer's action
(Tier C) and is a separate human ticket. GitHub redirects the old URL.

## Acceptance criteria for the spec

1. The design-impact check says "no design impact" on every ticket PR. Names change; results do not.
   No tolerance, snapshot value or oracle changes.
2. After the last build ticket, `git grep` finds the old names only in an allowlist: the shims, their
   tests, the legacy fixture, the CHANGELOG and historical ADRs. A check in the gates enforces this.
3. `pip install` of the built wheel gives `import owd`, `import channel_designer` (with the warning),
   `owd` and `cdesign` on the path.
4. The app builds as "Open Waterway Designer" with the new identifier, opens `.owd` and `.cdesign`, and saves `.owd`.
5. CI (`test`, `release-guard`, design impact, wheels, app bundles) is green on `main`.
6. A migration note in the CHANGELOG tells users what to change, and when the shims go.

## Out of scope

- Any change to engineering results, tolerances, rulesets or the oracle guard.
- The brand, look or layout of the app. Only its name and identifier change.
- Publishing to PyPI (issue #2 stays open). Docs use the new name.
- Removing the shims. A ticket exists for the release after this one.
- The GitHub repo rename itself (human ticket), and updating URLs after it.
- Changing the name of `ovcd-core` or the sister repo's links, other than noting them.

## Sequence (expand, migrate, contract)

Order matters because each step builds on the one before. Each ticket is one PR and leaves CI green.

1. **Crates (expand and migrate).** Rename the five crates, their directories and every reference to them
   in `Cargo.toml`, `xtask`, `tools/gates.sh`, workflows and docs. No behaviour change.
2. **Python package (expand).** Move the package to `owd`, rename the distribution to `open-waterway-designer`,
   point maturin at `owd._core`. Add the `channel_designer` shim. Update tests, the design-impact tool, examples and the wheels workflow.
3. **CLI (expand).** Name the binary `owd`. Keep `cdesign` as an alias binary with the deprecation note.
4. **Format (expand and migrate).** `.owd` is the saved extension. `.cdesign` still opens. Rename the schema and examples. Keep one legacy fixture.
5. **App (migrate).** New product name and bundle identifier, window title, file dialogs and file
   associations, end-to-end tests, bundle workflow.
6. **Docs.** GLOSSARY, README, CONTRIBUTING, ROADMAP, issue templates and the CHANGELOG migration note.
7. **Sweep.** A leftover-names check in the gates and CI, and a clean-up of what it finds.
8. **GitHub repo rename (human).** The maintainer runs one command.
9. **Point URLs at the new repo (after 8).** Cargo.toml, pyproject, schema `$id` and docs.
10. **Contract (a later release).** Remove the shims.

Tickets 1 to 7 are built in order by one builder. 8 waits for the maintainer. 9 waits for 8. 10 waits for the release.

## Risks

- **The rename can move a result.** A changed import path or module name can alter float formatting or ordering.
  The design-impact check and the full test suite guard this. Any moved number stops the build.
- **The bundle identifier changes the app's data folder** on each system. Saved window state and recent files do not carry over. Say so in the CHANGELOG.
- **Wheel names.** The wheel carries both `owd` and the `channel_designer` shim. The wheels workflow smoke tests must check both.
- **CI uses the base branch to compare.** The design-impact job installs the base and the PR. The scenario script must run on both sides, so ticket 2 must keep the `channel_designer` import working in the PR.
