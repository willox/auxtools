#!/usr/bin/env python3

from __future__ import annotations

import argparse
import re
from html.parser import HTMLParser
from pathlib import Path
from urllib.request import Request, urlopen


DEFAULT_VERSIONS = ("515", "516")
BASE_URL = "https://www.byond.com/docs/notes/{version}.html"


class NotesMarkdown(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.out: list[str] = []
        self.buf: list[str] = []
        self.href: str | None = None
        self.link: list[str] = []
        self.ul = 0
        self.li: list[dict[str, object]] = []
        self.bold = 0

    def escape(self, text: str) -> str:
        text = text.replace("\\", "\\\\")
        text = re.sub(r"(?<!\\)_", r"\\_", text)
        return text.replace("[", r"\[").replace("]", r"\]")

    def normalize(self, text: str) -> str:
        return re.sub(r"\s+", " ", text).strip()

    def blank(self) -> None:
        if self.out and self.out[-1] != "":
            self.out.append("")

    def line(self, text: str) -> None:
        if text:
            self.out.append(text)

    def add(self, text: str) -> None:
        if not text:
            return
        if self.href is not None:
            self.link.append(text)
            return
        if self.bold:
            text = f"**{text}**"
        if self.li:
            self.li[-1]["text"].append(text)  # type: ignore[index, union-attr]
        else:
            self.buf.append(text)

    def flush_buf(self, trailing_blank: bool = False) -> None:
        text = self.normalize("".join(self.buf))
        self.buf = []
        if text:
            self.line(text)
            if trailing_blank:
                self.blank()

    def flush_li(self) -> None:
        if not self.li:
            return
        item = self.li[-1]
        text_parts = item["text"]  # type: ignore[index]
        text = self.normalize("".join(text_parts))
        item["text"] = []
        if text:
            indent = "    " * max(self.ul - 1, 0)
            prefix = "    " * max(self.ul, 0) if item["emitted"] else indent + "-   "
            self.line(prefix + text)
            item["emitted"] = True

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attrs_dict = dict(attrs)
        if tag in ("title", "h1", "h3"):
            self.buf = []
        elif tag == "p":
            self.buf = []
        elif tag == "ul":
            if self.li:
                self.flush_li()
            elif self.buf:
                self.flush_buf(trailing_blank=False)
            self.ul += 1
        elif tag == "li":
            self.li.append({"text": [], "emitted": False})
        elif tag == "a":
            self.href = attrs_dict.get("href") or ""
            self.link = []
        elif tag in ("b", "strong"):
            self.bold += 1
        elif tag == "br":
            self.add("  \n")

    def handle_endtag(self, tag: str) -> None:
        if tag == "title":
            text = self.normalize("".join(self.buf))
            self.buf = []
            if text:
                self.line(text)
                self.blank()
        elif tag == "h1":
            text = self.normalize("".join(self.buf))
            self.buf = []
            if text:
                self.line("# " + text)
                self.blank()
        elif tag == "h3":
            text = self.normalize("".join(self.buf))
            self.buf = []
            if text:
                self.blank()
                self.line("### " + text)
                self.blank()
        elif tag == "p":
            if self.buf:
                self.flush_buf(trailing_blank=False)
        elif tag == "ul":
            self.ul = max(0, self.ul - 1)
            self.blank()
        elif tag == "li":
            self.flush_li()
            if self.li:
                self.li.pop()
        elif tag == "a":
            text = self.normalize("".join(self.link))
            href = self.href
            self.href = None
            self.link = []
            self.add(f"[{text}]({href})" if href else text)
        elif tag in ("b", "strong"):
            self.bold = max(0, self.bold - 1)

    def handle_data(self, data: str) -> None:
        self.add(self.escape(data))

    def markdown(self) -> str:
        while self.out and self.out[-1] == "":
            self.out.pop()
        return "\n".join(self.out) + "\n"


SECTION_RE = re.compile(r"^(Fixes|Features) \(\[More Info\]\(.+\)\)$")
PLAIN_HEADING_RE = re.compile(r"^[A-Za-z][A-Za-z0-9 &]+$")


def promote_headings(markdown: str) -> str:
    lines = markdown.splitlines()
    promoted: list[str] = []

    for index, line in enumerate(lines):
        next_line = lines[index + 1] if index + 1 < len(lines) else ""
        if SECTION_RE.match(line):
            promoted.append("#### " + line)
        elif PLAIN_HEADING_RE.match(line) and next_line.startswith("-   "):
            promoted.append("##### " + line)
        else:
            promoted.append(line)

    return "\n".join(promoted).rstrip() + "\n"


def fetch(version: str) -> str:
    request = Request(BASE_URL.format(version=version), headers={"User-Agent": "Mozilla/5.0"})
    with urlopen(request) as response:
        return response.read().decode("utf-8", "replace")


def convert(html: str) -> str:
    parser = NotesMarkdown()
    parser.feed(html)
    return promote_headings(parser.markdown())


def main() -> None:
    arg_parser = argparse.ArgumentParser(description="Fetch BYOND release notes and convert them to Markdown.")
    arg_parser.add_argument("versions", nargs="*", default=DEFAULT_VERSIONS, help="BYOND major versions to fetch")
    arg_parser.add_argument("--output-dir", type=Path, default=Path("changelogs"), help="Directory for generated Markdown files")
    args = arg_parser.parse_args()

    args.output_dir.mkdir(parents=True, exist_ok=True)

    for version in args.versions:
        html = fetch(version)
        output_path = args.output_dir / f"{version}.md"
        output_path.write_text(convert(html), encoding="utf-8")
        print(f"wrote {output_path}")


if __name__ == "__main__":
    main()
