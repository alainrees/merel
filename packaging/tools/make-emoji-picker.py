#!/usr/bin/env python3
"""Builds crates/ui-egui/assets/emoji/picker.txt, the emoji of the emoji picker in Signal's order
and groups, each with the words it is found by (Dutch and English).

Sources, downloaded by hand next to this script's arguments:
  emoji_16_0_ordering.json   github.com/googlefonts/emoji-metadata (Apache-2.0): order and groups
  nl.json, nld.json          cldr-json annotations (+ derived) for nl (Unicode License v3)
  en.json, end.json          the same for en

  python3 make-emoji-picker.py ordering.json nl.json nld.json en.json end.json noto-emoji-64.txt > picker.txt

Only emoji on the colour sheet (noto-emoji-64.txt) are kept. Lines: `@group`, then one emoji per
line, a tab, and its words, lower case and space separated.
"""
import json
import sys

ordering, nl, nld, en, end, sheet = sys.argv[1:7]
GROUPS = {
    "Smileys and emotions": "people",
    "People": "people",
    "Animals and nature": "animals",
    "Food and drink": "food",
    "Activities and events": "activities",
    "Travel and places": "travel",
    "Objects": "objects",
    "Symbols": "symbols",
    "Flags": "flags",
}
ORDER = ["people", "animals", "food", "activities", "travel", "objects", "symbols", "flags"]


def words(path, key):
    d = json.load(open(path, encoding="utf-8"))[key]["annotations"]
    return {e: v.get("default", []) + v.get("tts", []) for e, v in d.items()}


names = {}
for path, key in [(nl, "annotations"), (nld, "annotationsDerived"), (en, "annotations"), (end, "annotationsDerived")]:
    for e, w in words(path, key).items():
        names.setdefault(e.replace("️", ""), []).extend(w)

on_sheet = set()
for line in open(sheet, encoding="utf-8"):
    code = line.split(" ")[0]
    on_sheet.add("".join(chr(int(h, 16)) for h in code.split("-")))

groups = {g: [] for g in ORDER}
for group in json.load(open(ordering, encoding="utf-8")):
    for e in group["emoji"]:
        text = "".join(map(chr, e["base"]))
        if text not in on_sheet and text.replace("️", "") not in on_sheet:
            continue
        found = names.get(text.replace("️", ""), [])
        found += [s.strip(":").replace("_", " ") for s in e.get("shortcodes", [])]
        seen = []
        for w in " ".join(found).lower().replace(":", " ").replace(",", " ").split():
            if w not in seen:
                seen.append(w)
        groups[GROUPS[group["group"]]].append(text + "\t" + " ".join(seen))

out = []
for g in ORDER:
    out.append("@" + g)
    out.extend(groups[g])
sys.stdout.write("\n".join(out) + "\n")
