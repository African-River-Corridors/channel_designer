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
| `build.py` | Builds everything into `_site/`. |
| `check_links.py` | Fails on any broken internal link in `_site/`. |

## Build it locally

```bash
python3 website/build.py && python3 website/check_links.py
```
