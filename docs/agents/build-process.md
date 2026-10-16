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
3. **Local gates.** `bash tools/gates.sh` runs the fast gate: `cargo fmt --check`, clippy with warnings
   as errors, `cargo test --workspace` with Python made unusable, the Python tests, and the oracle guard.
   Then run `python3 tools/release_guard.py --history`, and the embargo check of the PR body (see
   `issue-tracker.md`).
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

## Mutation testing is a separate hardening pass (since 2026-10-16)

Builders do **not** run mutation tests during the build, before or after a merge. Parallel post-merge
runs competed with the builders and CI for the same machine, and none of their findings blocked a
ticket. Mutation testing runs as its own pass once the features of a spec are done and accepted:

- **When:** after the spec's feature tickets are merged and the maintainer is happy with them, and
  after that per release, on modules that have stopped changing.
- **How:** one run at a time, scoped to the modules under review (`cargo xtask mutants`), never in
  parallel with a build.
- **Output:** each surviving mutant that can change a public output becomes a fix-forward ticket
  (a sub-issue of the spec, labelled `ready-for-agent`). The fix adds a parity case regenerated with
  the reason "kills mutant ...". A survivor that cannot change an output gets a one-line equivalence
  argument in the pass report, not a ticket.
- Never stop another session's run by process name; use its process ID.

`cargo xtask mutants` stays in the repo for this pass. The harness's own check that the mutation task
works (from #22) is unchanged.

## Stop and report

The builder stops and reports to the coordinator at:

- 60% of the account's weekly token quota (set by the maintainer on 2026-10-16);
- any change to the tolerances (ADR 0005) or to the oracle guard;
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
- Never loosen a tolerance to make a number pass. Never hand-edit an expected value: a Reference Result
  is printed by the frozen Python (`cargo xtask regenerate`, with a recorded reason), or worked out and
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
- The frozen Python is tag `py-reference` (3fb5f2c). After the freeze it takes bug fixes only, each
  mirrored in the parity cases.

## Fixed-number tests: labels

A test that pins a number carries a label:

- `Label: sourced - <citation>`: the number comes from a standard, a published example or a hand
  calculation. It changes only when its source does.
- `Label: snapshot`: the number is what code printed. For the port it is what the frozen Python
  printed. It shows that the Rust core still agrees, not that the number is right.
- `Label: property`: no fixed number.

In Rust the label is a `//! Label:` line in the file header. Never hand-edit a snapshot value.
