"""Build the OWD website into website/_site/.

The site lives at https://www.africanrivercorridors.com/owd. The ARC site proxies /owd to this
repo's GitHub Pages site, so every URL in the output is an absolute /owd/... path (see
website/README.md for why).

Steps:
  1. Copy the static pages and assets to _site/.
  2. Assemble the docs Markdown into _docs/: the repo's own files, not copies kept in git.
     Links between the docs are kept; links to other repo files go to GitHub.
  3. Run mkdocs (website/mkdocs.yml) into _site/docs/.

Run from anywhere: python3 website/build.py
"""

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

STATIC = ["index.html", "site.css", "tokens.css", "favicon.svg", "fonts", "assets"]

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


def main() -> int:
    copy_static()
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
