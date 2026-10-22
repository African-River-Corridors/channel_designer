# Build process (spec 0001)

How spec 0001 (the parity harness and the Rust core port, issue #20) is built and merged. It follows
the process of the open-vessel-concept-drawings (OVCD) port, in **single-builder mode**: one builder
works the tickets one at a time, and adversarial review happens after the build, not on every ticket.

## The loop

The tickets are a task graph. Native GitHub "blocked by" links are the edges. The builder takes the next
ticket whose blockers are all closed, and no other. For each ticket:

1. **Read.** Read the ticket and its comments (`gh issue view <n> --comments`). Later comments may
   revise the acceptance criteria.
2. **Build.** Work in its own git worktree, on a branch `ticket/<n>-<slug>` cut from an up-to-date
   `origin/main`. Test first. One PR to `main` closes the ticket. Every acceptance criterion ticked in
   the PR body links to its evidence. Evidence must be reachable from the PR: a test name or file in the
   diff, a command and its output pasted in the body, or a CI job. A reviewer must not have to ask the
   builder for it.
3. **Local checks, targeted (since 2026-10-21).** CI is the full gate, so don't run the full
   `bash tools/gates.sh` before every push. Before pushing, run:
   - `cargo fmt --all --check`;
   - clippy with warnings as errors on the crates you touched;
   - the tests of the crates or Python modules you touched;
   - `python3 tools/release_guard.py --history`;
   - the embargo check of the PR body (see `issue-tracker.md`).

   This takes about a minute instead of about five. Run the full `tools/gates.sh` only when a change crosses
   many crates, such as a rename or a shared-type change, or when CI went red and you need to reproduce it.
4. **CI, without waiting on it.** The `test` and `release-guard` jobs must be green on the head commit
   before the merge, and branch protection enforces that. Nobody waits for them, though. As soon as the
   PR is open, turn on auto-merge (`gh pr merge <pr> --squash --auto --delete-branch`), then start the
   next ticket's reading and tests. If the PR goes red, fixing it comes before anything else. CI runs
   once per PR (pull request event); pushes to ticket branches do not trigger a second run.
5. **Merge.** Auto-merge squash-merges the PR when CI is green. Check that the PR is merged,
   the issue is closed, and `origin/main` holds the squash commit. Then write the **build notes** and
   remove the worktree. A local "main is already used by worktree" message is harmless.

   **Build notes** are one closing comment on the ticket: what was built, the commands run and their
   result, anything not done, and the PR link. Check the comment with the embargo one-liner first. The
   PR body stays the home of the evidence for each ticked criterion.
There is no mutation test and no per-ticket reviewer before a merge, and none after it either: see
below.

## Which CI jobs run where (since 2026-10-22)

Only `test` and `release-guard` are required, and they run on every pull request. The rest run only when
their inputs change, so a skipped job never blocks a merge:

| Job | On a pull request | On main | Other |
|---|---|---|---|
| CI `test`, `release-guard`, `drawing-crosscheck` | always | always | weekly, by hand |
| CI `app-linux`, `app-e2e` | `app/**`, the crates the app links, `examples/**`, Cargo files, toolchain, `ci.yml` | always | weekly, by hand |
| CI `web-parity`, `web-app` | the same, plus `crates/owd-web/**` and the web tools | always | weekly, by hand |
| App bundles (3 jobs) | `app/**`, the crates the app links, Cargo files, toolchain | the same paths | by hand |
| Wheels (5 jobs) | `crates/owd-py`, `owd-format`, `owd-core`, `src/**`, packaging files | the same paths | by hand |
| Website | `website/**`, `docs/**`, root `*.md`, `app/src/tokens.css` | the wider list in `website.yml` | by hand |
| Design impact | always | – | – |
| CodeQL (`codeql.yml`) | never | always | weekly, by hand |
| Nightly (every slow test) | its own files | – | daily, by hand |

A failed scheduled run (the nightly, the weekly CI run, CodeQL) opens an issue titled "Scheduled run
failed: <workflow>", or comments on the open one (`scheduled-failure.yml`). `tests/test_workflows.py`
checks that the required jobs are never filtered and that every scheduled workflow is watched.

CodeQL moves to `codeql.yml` only when the maintainer turns off the repository's CodeQL "default setup"
and sets the repository variable `CODEQL_ADVANCED_SETUP` to `true`. Until then the default setup still
runs on each pull request, and `codeql.yml` skips its job.

## Mutation testing is a separate hardening pass (since 2026-10-16)

Builders do **not** run mutation tests during the build, before or after a merge. Parallel post-merge
runs competed with the builders and CI for the same machine, and none of their findings blocked a
ticket. Mutation testing runs as its own pass once the features of a spec are done and accepted:

- **When:** after the spec's feature tickets are merged and the maintainer is happy with them, and
  after that per release, on modules that have stopped changing.
- **How:** one run at a time, scoped to the modules under review (`cargo xtask mutants`), never in
  parallel with a build.
- **Output:** each surviving mutant that can change a public output becomes a fix-forward ticket
  (a sub-issue of the spec, labelled `ready-for-agent`). The fix adds a test case with
  the reason "kills mutant ...". A survivor that cannot change an output gets a one-line equivalence
  argument in the pass report, not a ticket.
- Never stop another session's run by process name; use its process ID.

`cargo xtask mutants` stays in the repo for this pass. The harness's own check that the mutation task
works (from #22) is unchanged.

## Stop and report

The builder stops and reports to the coordinator at:

- 90% of the account's weekly token quota (raised from 60% by the maintainer on 2026-10-16);
- any change to the tolerance in `tools/design_impact.py` (ADR 0005);
- a ticket whose acceptance criteria cannot be met as written.

## After the build: adversarial review

Separate review sessions read what was merged: a break-it review (inputs where the Rust differs from
the Python, criteria claimed without evidence, gates that cannot fail, disclosure leaks), and a
two-axis review of the whole spec (repo standards, and the spec itself). Each verified finding becomes
a fix-forward issue.

## Stuck tickets

If one problem is still open after two honest attempts, label the ticket `ready-for-human`, comment why
in plain English (checked with the embargo one-liner), and move to the next ticket whose blockers are
closed. Tickets that depend on it wait.

## Rules every agent follows

- One PR per ticket, straight to `main`. Never push to `main` directly.
- Never loosen a tolerance to make a number pass. Never hand-edit an expected value: an expected value
  is worked out and
  cited.
- Tolerances are the ones in ADR 0005 and live in one constants file. Changing one stops the build.
- **Any check that compares numbers against the frozen Python compares by the ADR 0005 tolerance, never
  byte for byte.** The frozen Python prints different last bits on macOS (where it is generated) and on
  Linux (CI), so a byte check passes locally and fails in CI. This has happened twice (#23, #34). Prove
  each new check both ways in its PR: it passes on last-bit differences and fails on a real error.
- This repo is public. Check every issue, PR body and comment with the embargo one-liner in
  `issue-tracker.md`, and run the release guard with `--history` before every push. Write "closes #N" or
  "fixes #N" only for the ticket the PR really closes. New files need `git add -N` before the guard
  sees them.
- Several worktrees share one clone. Stage files by name, never with `git add -A` in a shared tree, and
  never touch another worktree.
- Never stop a process by name. Stop only a process you started, by its process id.
- Long builds and test runs go to the background with their output in a file that is polled.
- **Never edit `CHANGELOG.md` in a ticket PR (since 2026-10-22).** Add a changelog fragment,
  `changelog.d/<ticket>.<kind>.md` (kinds: `added`, `changed`, `deprecated`, `removed`, `fixed`, `security`,
  `design-impact`), with one Markdown entry and no heading. Every PR edited `CHANGELOG.md` before (28 of the
  last 30), so each merge put the other open PRs in conflict. Only the release PR touches `CHANGELOG.md`: it
  runs `python3 tools/changelog.py release <version>`, which moves `## Unreleased` and every fragment under
  the version heading and deletes the fragments. See `changelog.d/README.md`.
- The old pure-Python package is the tag `py-reference` (3fb5f2c), kept for history. The port is closed:
  a change that moves a design result needs a design-impact entry, a fragment
  `changelog.d/<ticket>.design-impact.md` (`tools/design_impact.py`).

## Fixed-number tests: labels

A test that pins a number carries a label:

- `Label: sourced - <citation>`: the number comes from a standard, a published example or a hand
  calculation. It changes only when its source does.
- `Label: snapshot`: the number is what code printed. For the port it is what the frozen Python
  printed. It shows that the Rust core still agrees, not that the number is right.
- `Label: property`: no fixed number.

In Rust the label is a `//! Label:` line in the file header. Never hand-edit a snapshot value.
