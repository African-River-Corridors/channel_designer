"""Build the OWD website into website/_site/.

The site lives at https://www.africanrivercorridors.com/owd. The ARC site proxies /owd to this
repo's GitHub Pages site, so every URL in the output is an absolute /owd/... path (see
website/README.md for why).

Steps:
  1. Copy the static pages and assets to _site/.
  2. Assemble the docs Markdown into _docs/: the repo's own files, not copies kept in git.
     Links between the docs are kept; links to other repo files go to GitHub.
  3. Run mkdocs (website/mkdocs.yml) into _site/docs/.

Optional: --app-dist <dir> adds the built web app (app/dist-web, from `npm run build:web`):
<dir>/index.html becomes _site/app.html (served at /owd/app) and the rest of <dir> goes to
_site/app-files/ (served at /owd/app-files/). Without it the "not in this build" app.html stays, so
the links to /owd/app still resolve in a local build.

Run from anywhere: python3 website/build.py [--app-dist app/dist-web]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

WEBSITE = Path(__file__).resolve().parent
REPO = WEBSITE.parent
SITE = WEBSITE / "_site"
DOCS_SRC = WEBSITE / "_docs"
BLOB = "https://github.com/African-River-Corridors/open-waterway-designer/blob/main/"
TREE = "https://github.com/African-River-Corridors/open-waterway-designer/tree/main/"

STATIC = ["index.html", "app.html", "site.css", "tokens.css", "favicon.svg", "fonts", "assets"]

# Repo file -> page in the docs. ADRs and specs keep their folder.
PAGES = {
    "README.md": "index.md",
    "ROADMAP.md": "roadmap.md",
    "GLOSSARY.md": "glossary.md",
    "CONTRIBUTING.md": "contributing.md",
    "CODE_OF_CONDUCT.md": "code-of-conduct.md",
    "SECURITY.md": "security.md",
    "TRADEMARKS.md": "trademarks.md",
    "CHANGELOG.md": "changelog.md",
    "docs/known-issues.md": "known-issues.md",
    "docs/design/style-guide.md": "style-guide.md",
    "docs/rules/channel-rules.md": "channel-rules.md",
    "docs/rules/codification.md": "codification.md",
}
for folder in ("docs/adr", "docs/specs"):
    for md in sorted((REPO / folder).glob("*.md")):
        PAGES[f"{folder}/{md.name}"] = f"{folder.removeprefix('docs/')}/{md.name}"

LINK = re.compile(r"(\]\()([^)\s]+)(\))")


def rewrite_link(target: str, source: str, dest: str) -> str:
    if re.match(r"^([a-z]+:|#|/)", target):
        return target
    path, _, anchor = target.partition("#")
    resolved = os.path.normpath(os.path.join(os.path.dirname(source), path))
    anchor = f"#{anchor}" if anchor else ""
    if resolved in PAGES:
        rel = os.path.relpath(PAGES[resolved], os.path.dirname(dest) or ".")
        return rel + anchor
    if (REPO / resolved).is_dir():
        return TREE + resolved + anchor
    if (REPO / resolved).exists():
        return BLOB + resolved + anchor
    return target  # mkdocs --strict reports it


def assemble_docs() -> None:
    shutil.rmtree(DOCS_SRC, ignore_errors=True)
    for source, dest in PAGES.items():
        text = (REPO / source).read_text(encoding="utf-8")
        text = LINK.sub(lambda m: m[1] + rewrite_link(m[2], source, dest) + m[3], text)
        out = DOCS_SRC / dest
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text, encoding="utf-8")
    assets = DOCS_SRC / "assets"
    shutil.copytree(WEBSITE / "assets" / "logo", assets / "logo")
    shutil.copytree(WEBSITE / "fonts", assets / "fonts")  # tokens.css finds them at fonts/
    shutil.copy(WEBSITE / "tokens.css", assets / "tokens.css")
    shutil.copy(WEBSITE / "docs-theme" / "owd.css", assets / "owd.css")
    shutil.copy(WEBSITE / "favicon.svg", assets / "favicon.svg")


def copy_static() -> None:
    shutil.rmtree(SITE, ignore_errors=True)
    SITE.mkdir()
    for name in STATIC:
        src = WEBSITE / name
        if src.is_dir():
            shutil.copytree(src, SITE / name)
        else:
            shutil.copy(src, SITE / name)


def add_app(dist: Path) -> None:
    """Put the web app build where the ARC proxy needs it.

    The page is app.html, not app/index.html, and the files go in app-files/, not app/: a folder
    named app/ next to app.html makes GitHub Pages redirect /app to /app/ on its own host.
    """
    shutil.copy(dist / "index.html", SITE / "app.html")
    files = SITE / "app-files"
    for src in sorted(dist.rglob("*")):
        rel = src.relative_to(dist)
        if src.is_file() and rel != Path("index.html"):
            (files / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(src, files / rel)


def main(argv: list[str] | None = None) -> int | str:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--app-dist", type=Path, help="built web app folder (app/dist-web)")
    args = parser.parse_args(argv)
    if args.app_dist is not None and not (args.app_dist / "index.html").is_file():
        return f"build.py: no index.html in --app-dist {args.app_dist}. Build the app first (npm run build:web)."
    copy_static()
    if args.app_dist is not None:
        add_app(args.app_dist)
    assemble_docs()
    subprocess.run(
        [sys.executable, "-m", "mkdocs", "build", "--strict", "--clean",
         "-f", str(WEBSITE / "mkdocs.yml"), "-d", str(SITE / "docs")],
        check=True,
    )
    # GitHub Pages serves only the root 404.html for a project site.
    shutil.copy(SITE / "docs" / "404.html", SITE / "404.html")
    (SITE / ".nojekyll").touch()
    print(f"Built {SITE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
