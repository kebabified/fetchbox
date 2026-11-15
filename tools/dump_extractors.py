"""Exports yt-dlp's site URL patterns to assets/extractors.json, so the Rust helper can tell
(offline, instantly) whether a link is something yt-dlp can download.

Run from the Fetchbox folder after big yt-dlp updates (needs `pip install yt-dlp`):

    python tools/dump_extractors.py
"""
import json
import os
import re
from pathlib import Path

os.environ["YTDLP_NO_LAZY_EXTRACTORS"] = "1"  # load the real classes, so custom rules can be detected

import yt_dlp
from yt_dlp.extractor import gen_extractor_classes
from yt_dlp.extractor.common import InfoExtractor

BS = "\\"
out = []
for ie in gen_extractor_classes():
    key = ie.ie_key()
    if key == "Generic":
        continue
    pats = ie._VALID_URL
    if not pats:
        continue
    if isinstance(pats, str):
        pats = [pats]
    # does this extractor (or a parent below InfoExtractor) add its own rules on top of the URL pattern?
    custom = next(c for c in ie.__mro__ if "suitable" in c.__dict__) is not InfoExtractor
    fixed = []
    for p in pats:
        p = p.replace(BS + "Z", BS + "z")                         # end-of-text is spelled \z in Rust
        p = re.sub(r"(?<!\\)((?:\\\\)*)\\([<>])", lambda m: m.group(1) + m.group(2), p)  # \< \> are plain < > in Python
        p = re.sub(r"\{,(\d+)\}", lambda m: "{0," + m.group(1) + "}", p)                  # {,n} quantifier
        fixed.append(p)
    out.append({"k": key, "p": fixed, "c": custom})

dest = Path(__file__).resolve().parent.parent / "assets" / "extractors.json"
dest.parent.mkdir(exist_ok=True)
dest.write_text(json.dumps({"yt_dlp": yt_dlp.version.__version__, "extractors": out},
                           ensure_ascii=False, separators=(",", ":")), "utf-8")
print(len(out), "extractors ->", dest, f"({dest.stat().st_size // 1024} KB)")
