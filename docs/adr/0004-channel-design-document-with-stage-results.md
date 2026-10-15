---
status: accepted
---
# One Channel design document, shared by the CLI and the app, holding stage results

A channel design is a JSON document (`.cdesign`) with a published schema. Each CLI subcommand
reads one and writes a richer one, and the desktop app opens and saves the same file. The
document holds the inputs and every stage's results. Each stage records a hash of what it
depended on, so an edit marks the later stages stale instead of silently leaving old numbers.
Small geometry (bank lines, river polygon, design lines) is embedded. The DEM is referenced by
path and content hash, never embedded, because it can be gigabytes.
