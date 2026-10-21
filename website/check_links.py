"""Check the built site (website/_site/) for broken internal links.

Every internal URL must be an absolute /owd/... path (a relative URL breaks at /owd with no
trailing slash) and must resolve to a built file the way GitHub Pages resolves it: the file
itself, or the file plus ".html". A link with a #fragment must land on an id in the target page.
External links are not fetched.

Run after build.py: python3 website/check_links.py
"""

import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

SITE = Path(__file__).resolve().parent / "_site"
PREFIX = "/owd"
EXTERNAL = re.compile(r"^([a-z][a-z0-9+.-]*:|//)", re.I)
# The two folder URLs the ARC proxy maps to an index page (see website/README.md).
INDEXES = {"/owd": "index.html", "/owd/docs": "docs/index.html"}


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.urls: list[str] = []
        self.ids: set[str] = set()

    def handle_starttag(self, tag, attrs):
        for name, value in attrs:
            if name == "id" and value:
                self.ids.add(value)
            if name in ("href", "src") and value is not None:
                self.urls.append(value)


def parse(path: Path) -> Page:
    page = Page()
    page.feed(path.read_text(encoding="utf-8"))
    return page


def resolve(path: str) -> Path | None:
    if path in INDEXES:
        return SITE / INDEXES[path]
    rel = path.removeprefix(PREFIX + "/")
    for candidate in (SITE / rel, SITE / f"{rel}.html"):
        if candidate.is_file():
            return candidate
    return None


def main() -> int:
    pages = {p: parse(p) for p in SITE.rglob("*.html")}
    errors = []
    for path, page in pages.items():
        here = path.relative_to(SITE)
        for url in page.urls:
            if EXTERNAL.match(url) or url == "#":
                continue
            parts = urlsplit(url)
            if url.startswith("#"):
                frag = unquote(parts.fragment)
                if not frag.startswith("__") and frag not in page.ids:
                    errors.append(f"{here}: no id for {url}")
                continue
            if not parts.path.startswith(PREFIX + "/") and parts.path != PREFIX:
                errors.append(f"{here}: not an absolute {PREFIX}/ URL: {url}")
                continue
            target = resolve(parts.path)
            if target is None:
                errors.append(f"{here}: broken link {url}")
            elif parts.fragment and target.suffix == ".html":
                ids = pages[target].ids if target in pages else parse(target).ids
                if unquote(parts.fragment) not in ids:
                    errors.append(f"{here}: no id for {url}")
    for e in sorted(set(errors)):
        print(e)
    print(f"Checked {len(pages)} pages: {len(set(errors))} broken link(s).")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
