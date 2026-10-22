- **Changelog fragments (contributors).** A pull request no longer edits `CHANGELOG.md`. It adds one file,
  `changelog.d/<pr>.<kind>.md`, so two open pull requests never conflict over the changelog.
  `tools/changelog.py release <version>` moves the fragments and any `## Unreleased` entries under the new
  version at release time; `tools/changelog.py check` checks them. The design-impact check accepts a
  `changelog.d/<pr>.design-impact.md` fragment as well as a bullet under `## Unreleased` > `### Design impact`.
