"""Make the small full-body portraits the Modding page bundles.

The website's portraits are 7000x7000 transparent PNGs. This downloads each
one, crops it to the figure, fits it into 180x240 and writes
assets/roster/<file>.png. Run with Pillow installed:

    python scripts/build-roster-portraits.py [download-folder]
"""

import io
import os
import sys
import urllib.request

from PIL import Image

Image.MAX_IMAGE_PIXELS = None

BASE = "https://assets.modded-evolve.com/images/full_body_portraits/"
FILES = [
    "goliath", "goliathmeteor", "kraken", "krakenelder", "wraith", "behemoth",
    "rockybobcicle", "gorgon", "markov", "markovblitz", "hyde", "parnell",
    "torvald", "lennox", "val", "valrogue", "lazarus", "caira", "cairaquantum",
    "slim", "emet", "parnelltank", "hank", "hanksgt", "bucket", "cabot",
    "cabotbattle", "sunny", "kala", "maggiedaisy", "maggiewastelanddaisy",
    "griffin", "griffinelectro", "abe", "aberenegade", "crow", "jack",
]
SIZE = (180, 240)

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = os.path.join(root, "assets", "roster")
cache = sys.argv[1] if len(sys.argv) > 1 else None
os.makedirs(out, exist_ok=True)
for name in FILES:
    cached = cache and os.path.join(cache, name + ".png")
    if cached and os.path.exists(cached):
        data = open(cached, "rb").read()
    else:
        request = urllib.request.Request(BASE + name + ".png", headers={"User-Agent": "Mozilla/5.0"})
        data = urllib.request.urlopen(request).read()
        if cached:
            open(cached, "wb").write(data)
    figure = Image.open(io.BytesIO(data)).convert("RGBA")
    figure = figure.crop(figure.split()[-1].getbbox())
    figure.thumbnail(SIZE, Image.LANCZOS)
    canvas = Image.new("RGBA", SIZE, (0, 0, 0, 0))
    # Feet on the bottom edge, centred across.
    canvas.paste(figure, ((SIZE[0] - figure.width) // 2, SIZE[1] - figure.height))
    canvas.save(os.path.join(out, name + ".png"), optimize=True)
    print(name, os.path.getsize(os.path.join(out, name + ".png")))
