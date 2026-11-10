#!/usr/bin/env python3
"""Write reeve-seal.svg: an uneven wax disc with a dotted ring and the tally r cut out of it."""
import math
import pathlib

MARK = ("M13 9a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v19H13zM13 32h16v6H13zM13 42h16v6H13z"
        "M13 52h16v3a3 3 0 0 1-3 3H16a3 3 0 0 1-3-3zM33 24A18 18 0 0 1 51 6v16a2 2 0 0 1-2 2z")

points = []
for i in range(96):
    t = 2 * math.pi * i / 96
    r = 28.2 + 1.3 * math.sin(7 * t + 0.4) + 0.8 * math.sin(11 * t + 2.1) + 0.5 * math.sin(3 * t)
    points.append(f"{32 + r * math.cos(t):.2f},{32 + r * math.sin(t):.2f}")

svg = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">'
    '<mask id="reeve-seal-cut" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">'
    f'<polygon points="{" ".join(points)}" fill="#fff"/>'
    '<circle cx="32" cy="32" r="22.5" fill="none" stroke="#000" stroke-width="1.5" stroke-dasharray="2 3"/>'
    f'<path transform="translate(15 15) scale(0.53125)" d="{MARK}" fill="#000"/>'
    '</mask>'
    '<rect width="64" height="64" fill="currentColor" mask="url(#reeve-seal-cut)"/>'
    '</svg>\n'
)
pathlib.Path(__file__).with_name("reeve-seal.svg").write_text(svg)
