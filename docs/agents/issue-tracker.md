# Issue tracker: GitHub

Issues and specs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body-file body.md`.
- **Read an issue**: `gh issue view <number> --comments`. Later comments may revise a ticket.
- **List issues**: `gh issue list --state open --label <label>`.
- **Comment**: `gh issue comment <number> --body-file comment.md`
- **Labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

## This repo is public

Every issue, comment and PR body is public. Never put confidential project names, client names,
site data, people's names or internal decision IDs in them. The release guard scans only committed
files, so check each body before you publish it:

```
python3 -c "import sys; sys.path.insert(0, 'tools'); import release_guard as g; hits = [(t, l) for l in open(sys.argv[1]) for t, p in g.TERMS.items() if p.search(l)]; print(len(hits), 'hit(s)'); [print(t, '|', l.strip()) for t, l in hits]; sys.exit(1 if hits else 0)" body.md
```

Publish only when it reports 0 hits.

## Specs and tickets

- A **spec** is one issue titled `Spec NNNN: ...`. Its tickets are GitHub **sub-issues** of it.
- **Blocking** uses GitHub's native issue dependencies:
  `gh api --method POST repos/<owner>/<repo>/issues/<ticket>/dependencies/blocked_by -F issue_id=<blocker-db-id>`,
  where `<blocker-db-id>` is the numeric database id (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`),
  not the `#number`. A ticket is unblocked when every blocker is closed.
- **Frontier**: the open sub-issues of the spec with no open blocker and no assignee, in issue order.
- One PR closes one ticket. Never close or edit the spec issue from a ticket.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.
