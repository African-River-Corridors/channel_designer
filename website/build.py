"""Build the OWD website into website/_site/.

The site lives at https://www.africanrivercorridors.com/owd. The ARC site proxies /owd to this
repo's GitHub Pages site, so every URL in the output is an absolute /owd/... path (see
website/README.md for why).

Steps:
  1. Copy the static pages and assets to _site/.
  2. Put a placeholder at _site/docs/index.html (ticket 3 of spec 0010 replaces it with the docs).

Run from anywhere: python3 website/build.py
"""

import shutil
import sys
from pathlib import Path

WEBSITE = Path(__file__).resolve().parent
SITE = WEBSITE / "_site"

STATIC = ["index.html", "site.css", "tokens.css", "favicon.svg", "fonts", "assets"]


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
    (SITE / "docs").mkdir()
    shutil.copy(WEBSITE / "docs-placeholder.html", SITE / "docs" / "index.html")
    (SITE / ".nojekyll").touch()
    print(f"Built {SITE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
