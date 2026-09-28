#!/usr/bin/env python3
"""Rasterize Natural Earth's 110m land polygons (public domain) into
data/land-mask.txt: one row per 2 degrees of latitude from 75 N to 57 S,
one column per 2 degrees of longitude from 180 W, '#' where the cell's
centre is on land. The Day and night group's map draws it as dots
(src/settings/location_map.rs).

    curl -sSfLO https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_110m_land.geojson
    dev/land-mask.py ne_110m_land.geojson > data/land-mask.txt
"""
import json
import sys

NORTH, SOUTH, STEP = 75, -57, 2

rings = []
for feature in json.load(open(sys.argv[1]))["features"]:
    geom = feature["geometry"]
    polys = geom["coordinates"] if geom["type"] == "MultiPolygon" else [geom["coordinates"]]
    for poly in polys:
        rings.extend(poly)
boxes = [
    (min(p[0] for p in r), max(p[0] for p in r), min(p[1] for p in r), max(p[1] for p in r), r)
    for r in rings
]


def inside(x, y):
    """Even-odd over every ring: holes (lakes) cancel their land."""
    hit = False
    for x0, x1, y0, y1, r in boxes:
        if not (x0 <= x <= x1 and y0 <= y <= y1):
            continue
        j = len(r) - 1
        for i in range(len(r)):
            xi, yi = r[i]
            xj, yj = r[j]
            if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
                hit = not hit
            j = i
    return hit


lat = NORTH - STEP / 2
while lat > SOUTH:
    print("".join("#" if inside(-180 + STEP / 2 + k * STEP, lat) else "." for k in range(360 // STEP)))
    lat -= STEP
