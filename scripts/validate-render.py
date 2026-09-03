#!/usr/bin/env python3
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "release" / "stack"
FIXTURE = ROOT / "tests" / "fixtures" / "render.stack"


completed = subprocess.run(
    [BINARY, "render", FIXTURE],
    check=False,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)
if completed.returncode != 0:
    raise SystemExit(completed.stderr.decode("utf-8", errors="replace"))
if completed.stderr:
    raise SystemExit("render smoke emitted unexpected diagnostics")

root = ET.fromstring(completed.stdout)
if root.tag != "{http://www.w3.org/2000/svg}svg":
    raise SystemExit("render output is not an SVG root element")
if not root.attrib.get("viewBox"):
    raise SystemExit("render output has no viewBox")

print("validated CLI standalone SVG")
