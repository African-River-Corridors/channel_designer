# Spec 0010: The OWD website and docs

Status: **approved to build by the maintainer, 2026-10-21.** Going live is a separate step that the
maintainer approves (see "Go live").

## Problem statement

OWD has a brand (spec 0008) but no public face. The sibling tool OVCD already runs a website and docs at
`africanrivercorridors.com/ovcd`. OWD needs the same at `africanrivercorridors.com/owd`, with the docs at
`/owd/docs`. The web app comes later, at `/owd/app`.

## Solution

Copy the OVCD pattern. One folder, `website/`, builds into `website/_site/`. A GitHub Pages site serves
it. The ARC company site proxies `/owd` to that Pages site, so there is one source and no copy.

- **Home page.** Plain HTML and CSS on `tokens.css`. No build step for the page, no script, no
  third-party request. The three fonts are served from the site.
- **Docs.** mkdocs-material, themed with the tokens. The pages are the repo's own Markdown, assembled
  at build time: README, ROADMAP, GLOSSARY, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, the style
  guide, `docs/known-issues.md`, `docs/adr/` and `docs/specs/`. A new ADR or spec joins the nav by
  itself. Links between these files stay working; a link to any other repo file goes to GitHub.
- **Build.** `python3 website/build.py` writes `website/_site/`. `mkdocs --strict` fails on a broken
  docs link. `website/check_links.py` fails on any broken internal link in the built site.
- **Workflow.** `.github/workflows/website.yml` builds and checks on a pull request that touches the site
  or docs (a `paths:` filter), so a change is proved before it merges. On `main` it also deploys to GitHub
  Pages. Action versions are pinned by SHA.

## URL rules (the ARC proxy)

The ARC site redirects `/path/` to `/path` and `/page.html` to `/page`. So:

- Every internal URL is an absolute `/owd/...` path, with no trailing slash and no `.html`.
- Only two folder URLs map to an index page: `/owd` and `/owd/docs`. Other pages are named files.
- The web app, when it comes, is `app.html` (served as `/owd/app`) with its files under `app-files/`,
  never a folder named `app/`. This spec does not add the app, and the home page does not link it.

## Claims on the page match what is built

Every sentence on the home page is true of the code in the repo today.

- PyPI is not published (issue #2). The install text must not say `pip install open-waterway-designer`
  works. It shows what works today: install from the git URL (needs a Rust toolchain), or build the
  desktop app from source. It says a PyPI release is coming.
- There is no web app. The home page has no "Web app" link.
- The desktop app has no release yet. The page says it builds from source and that CI builds unsigned
  pre-release bundles.

## Files

| Path | What |
|---|---|
| `website/index.html`, `website/site.css` | The home page (from the approved brand pack) |
| `website/build.py`, `website/check_links.py` | Build and link check |
| `website/mkdocs.yml`, `website/hooks.py`, `website/overrides/`, `website/docs-theme/owd.css` | The docs |
| `website/requirements.txt` | Exact pins for the docs tools |
| `website/README.md` | How it is served and how to build it |
| `.github/workflows/website.yml` | Build, check, deploy |

`website/tokens.css` is owned by the app restyle (spec 0009). It stays byte-identical to
`app/src/tokens.css`. The site copies it; it never edits it.

## Tickets

1. **Home page and build.** `index.html`, `site.css`, `build.py` (static copy), `check_links.py`.
2. **Build and link-check workflow**, with a `pull_request` trigger and a `paths:` filter. Blocked by 1.
3. **Docs theme over the repo Markdown.** mkdocs, hooks, theme, pins, fonts in the docs. Blocked by 2.
4. **Deploy job** to GitHub Pages on `main`. Blocked by 3.

## Go live (not part of the build)

Two steps publish the site. The maintainer approves each; no ticket does them.

1. Enable GitHub Pages on the repo with source "GitHub Actions".
2. Add `/owd`, `/owd/docs` and `/owd/:path*` rewrites to the ARC site's `vercel.json`, the same three
   lines as `/ovcd`.

## Acceptance

- `python3 website/build.py && python3 website/check_links.py` exits 0 on a clean checkout.
- The pull request check runs on a PR that touches `website/` and fails on a broken link.
- The built home page and docs show in light and dark, at phone and desktop width, with no request to
  another host.
- The check for `website/tokens.css` still passes (`cd app && npm run contrast`).

## Out of scope

- The web app page and its build (a later spec, after the web app exists).
- Search-engine work, analytics and a cookie banner (the site sets no cookie).
- Publishing: the two go-live steps above.
