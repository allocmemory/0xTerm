"""Adds a release to the Sparkle feed (0xTermSite's static/appcast.xml).

    python3 packaging/appcast.py <appcast.xml> <Info.plist> <dmg> '<sign_update output>'

The newest release goes first. Its release notes are that version's section of
CHANGELOG.md, as HTML. Running it again for the same version replaces the entry.
"""

import html
import plistlib
import re
import sys
from email.utils import formatdate
from pathlib import Path

REPO = "https://github.com/allocmemory/0xTerm"

HEADER = """<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>0xTerm</title>
    <link>https://0xterm.org/appcast.xml</link>
    <description>0xTerm releases</description>
    <language>en</language>
  </channel>
</rss>
"""


def changelog_section(version: str) -> str:
    text = Path(__file__).resolve().parent.parent.joinpath("CHANGELOG.md").read_text()
    match = re.search(rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not match:
        sys.exit(f"appcast: CHANGELOG.md has no section for {version}")
    return match.group(1).strip()


def inline(text: str) -> str:
    text = html.escape(text)
    text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', text)
    return text


def to_html(markdown: str) -> str:
    """Just the Markdown the changelog uses: ### headings, - lists, paragraphs."""
    out, items, para = [], [], []

    def flush():
        if para:
            out.append(f"<p>{inline(' '.join(para))}</p>")
            para.clear()
        if items:
            out.append("<ul>" + "".join(f"<li>{inline(i)}</li>" for i in items) + "</ul>")
            items.clear()

    for line in markdown.splitlines():
        stripped = line.strip()
        if not stripped:
            flush()
        elif stripped.startswith("### "):
            flush()
            out.append(f"<h3>{inline(stripped[4:])}</h3>")
        elif stripped.startswith("- "):
            if para:
                flush()
            items.append(stripped[2:])
        elif items and line.startswith("  "):
            items[-1] += " " + stripped
        else:
            para.append(stripped)
    flush()
    return "\n".join(out)


def main() -> None:
    appcast, info_plist, dmg, signature = sys.argv[1:5]
    with open(info_plist, "rb") as f:
        info = plistlib.load(f)
    version, build = info["CFBundleShortVersionString"], info["CFBundleVersion"]
    minimum = info["LSMinimumSystemVersion"]
    dmg_name = Path(dmg).name
    if not re.fullmatch(r'sparkle:edSignature="[^"]+" length="\d+"', signature.strip()):
        sys.exit(f"appcast: unexpected sign_update output: {signature!r}")

    item = f"""    <item>
      <title>0xTerm {version}</title>
      <pubDate>{formatdate(usegmt=True)}</pubDate>
      <sparkle:version>{build}</sparkle:version>
      <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{minimum}</sparkle:minimumSystemVersion>
      <link>{REPO}/releases/tag/v{version}</link>
      <description><![CDATA[
{to_html(changelog_section(version))}
      ]]></description>
      <enclosure url="{REPO}/releases/download/v{version}/{dmg_name}" type="application/octet-stream" {signature.strip()}/>
    </item>
"""
    path = Path(appcast)
    feed = path.read_text() if path.exists() else HEADER
    # Drop an earlier entry for this version, then put this one first.
    feed = re.sub(
        rf"    <item>\n(?:(?!    </item>).)*?<sparkle:shortVersionString>{re.escape(version)}</sparkle:shortVersionString>.*?    </item>\n",
        "",
        feed,
        flags=re.S,
    )
    anchor = "    <language>en</language>\n"
    if anchor not in feed:
        sys.exit(f"appcast: {appcast} doesn't look like this script's feed")
    feed = feed.replace(anchor, anchor + item, 1)
    path.write_text(feed)
    print(f"added {version} (build {build}) to {appcast}")


if __name__ == "__main__":
    main()
