# Build lanes

How a build lane (one agent session working a list of tickets) works in this repo. A lane brief names
its tickets and points here for the rest.

## Set up

- **Worktree.** Work in your own git worktree, one branch per PR:
  `git worktree add ../owd-<lane> -b <branch> origin/main`. Leave other lanes' worktrees alone.
- **Toolchain.** `rust-toolchain.toml` pins Rust. If `cargo` is not on `PATH`, add the pinned toolchain's
  `bin` folder from `rustup toolchain list -v`.
- **Build folder.** Set `CARGO_TARGET_DIR` to a folder of your own (for example
  `$HOME/Dev/_cargo-target/owd-<lane>`), so lanes never share or fight over one build.
- **Disk.** Keep more than 20 GB free (`df -h ~`). Delete your build folder when you hand over.

## Each PR

1. Write the test first. One PR per ticket.
2. Add one changelog fragment (`changelog.d/README.md`), plus a `design-impact` fragment when a design
   result moves.
3. Turn on auto-merge (`gh pr merge <n> --auto --squash`). Required checks: `test`, `release-guard`,
   and, on a PR that touches the app, `app-e2e` and `web-app`. These two skip themselves on other PRs.
4. After a merge that touches the app, check main's `app-e2e` and `web-app`. If your change turned them
   red, fix that before the next ticket.

## The app slot

Only one lane at a time has an open PR that touches `app/`, `crates/owd-shell`, `crates/owd-web` or
`app/src-tauri`. Ask the coordinator for the slot, and say when your app PR has merged and its checks
pass.

## Release hold

A release is cut from a release branch (`docs/agents/releasing.md`), so main keeps merging during a
release. A lane does not hold PRs for a release unless the coordinator asks.

## Hand-over

- At 85 % context, finish the PR you are on, write a hand-over note (state, open PRs, next steps,
  traps), and start nothing new.
- Report each PR to the coordinator in one line.
- If you wait for a CI result and nothing arrives within 15 minutes, read the PR's state directly
  (`gh pr checks <n>`); a background notice can be lost.
