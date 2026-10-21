# Website and docs

The OWD website. Live at **https://www.africanrivercorridors.com/owd** once it is switched on (docs at
`/owd/docs`). Spec 0010 (`docs/specs/0010-website.md`).

## How it is served

- `.github/workflows/website.yml` builds this folder. A pull request that touches the site builds and
  checks the links. A push to `main` also deploys to GitHub Pages.
- The ARC website (Vercel) proxies `/owd` and `/owd/*` to the Pages site with rewrites. There is one
  source, this folder, and no copy on the ARC side.
- The ARC site redirects `/path/` to `/path` and `/page.html` to `/page`. So every URL here is an
  absolute `/owd/...` path with no trailing slash and no `.html`. A relative URL would resolve against
  the wrong folder at `/owd`. GitHub Pages serves `page.html` for `/page`.
- The ARC rewrites map two folder URLs to their index page: `/owd` and `/owd/docs`. Do not add other
  folder index pages; name the page instead.
- The web app does not exist yet. When it does it is `app.html` (served as `/owd/app`) with its files
  in `app-files/`, never a folder named `app/`: that makes GitHub Pages redirect `/app` to `/app/` on its
  own host, which breaks the proxy.
- No third-party request: the fonts are in `fonts/`.
- Claims on the page match what is built. PyPI is not published, so the install text shows the git
  install.

## Layout

| Path | What it is |
| --- | --- |
| `index.html`, `site.css` | The home page. Plain HTML and CSS, no script. |
| `tokens.css`, `fonts/` | The brand tokens and fonts. `tokens.css` is owned by the app (`app/src/tokens.css`) and stays byte-identical to it. |
| `assets/`, `favicon.svg` | Logo and illustrations. Copied from the brand pack; do not edit them here. |
| `mkdocs.yml`, `hooks.py`, `overrides/` | The docs: mkdocs-material with the OWD theme. |
| `docs-theme/owd.css` | The docs theme over `tokens.css`. |
| `requirements.txt` | Exact pins for the docs tools. mkdocs stays below 2.0, which drops the plugin and theme systems this build uses. |
| `build.py` | Builds everything into `_site/`. |
| `check_links.py` | Fails on any broken internal link in `_site/`. |

## Build it locally

The docs are the repo's own Markdown: README, ROADMAP, GLOSSARY, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY,
TRADEMARKS, CHANGELOG, the style guide, `docs/known-issues.md`, `docs/adr/` and `docs/specs/`. `build.py`
copies them into `_docs/` at build time. Edit the files where they are; never in `_docs/`. A new ADR or
spec joins the nav by itself. Links between these files keep working; a link to any other repo file goes
to GitHub.

```bash
python3 -m venv website/.venv && website/.venv/bin/pip install -r website/requirements.txt
website/.venv/bin/python website/build.py && python3 website/check_links.py
```
