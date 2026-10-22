# Changelog fragments

A pull request does not edit `CHANGELOG.md`. It adds one small file here instead, so two pull requests
never conflict over the changelog.

Name the file `<id>.<kind>.md`:

- `<id>` is the issue or pull request number, such as `294`. Use a short slug if there is neither.
- `<kind>` is one of `added`, `changed`, `deprecated`, `removed`, `fixed`, `security` or `design-impact`.

Write one entry in Markdown: a bullet (`- **Title.** What changed and why.`) or plain text. No headings.
Add one file per kind if a pull request needs more than one.

A change that moves a design result needs a `design-impact` fragment that says what moved, why and which
way. The `Design impact` check reads it (`tools/design_impact.py`).

`python3 tools/changelog.py check` checks the files. At release time,
`python3 tools/changelog.py release <version>` moves every entry into `CHANGELOG.md` under the new version
and deletes the fragments.
