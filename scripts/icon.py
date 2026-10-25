"""Builds assets/app.ico and assets/app.res from assets/icon.svg (40 px and up)
and assets/icon-small.svg (16-32 px). Run after changing either SVG:
python scripts/icon.py (needs Pillow, Chrome and the Windows SDK's rc.exe).

Each size is rasterized on its own by headless Chrome, so small sizes stay sharp
instead of being scaled down from the large one.
"""
import glob
import io
import struct
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
CHROME = r"C:\Program Files\Google\Chrome\Application\chrome.exe"
SIZES = [16, 20, 24, 32, 40, 48, 64, 256]


def render() -> dict[int, Image.Image]:
    # All sizes side by side in one screenshot, then cropped apart.
    x, cells = 0, []
    for s in SIZES:
        svg = (ASSETS / ("icon-small.svg" if s <= 32 else "icon.svg")).as_uri()
        cells.append(f'<img src="{svg}" style="position:absolute;left:{x}px;top:0;width:{s}px;height:{s}px">')
        x += s + 8
    with tempfile.TemporaryDirectory() as tmp:
        page, shot = Path(tmp) / "sheet.html", Path(tmp) / "sheet.png"
        page.write_text("<body style='margin:0'>" + "".join(cells) + "</body>", encoding="utf-8")
        subprocess.run([CHROME, "--headless=new", "--hide-scrollbars", "--force-device-scale-factor=1",
                        f"--user-data-dir={Path(tmp) / 'chrome'}", "--default-background-color=00000000",
                        f"--window-size={x},{max(SIZES)}", f"--screenshot={shot}", page.as_uri()],
                       check=True, capture_output=True)
        sheet = Image.open(shot).convert("RGBA")
    out, x = {}, 0
    for s in SIZES:
        out[s] = sheet.crop((x, 0, x + s, s))
        x += s + 8
    return out


def write_ico(images: dict[int, Image.Image], path: Path):
    # PNG data for every size; Windows reads that since Vista.
    blobs = []
    for s, im in images.items():
        b = io.BytesIO()
        im.save(b, "PNG")
        blobs.append((s, b.getvalue()))
    head = struct.pack("<HHH", 0, 1, len(blobs))
    offset, dirs = 6 + 16 * len(blobs), b""
    for s, data in blobs:
        dirs += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    path.write_bytes(head + dirs + b"".join(d for _, d in blobs))


def main():
    images = render()
    write_ico(images, ASSETS / "app.ico")
    # Preview on light and dark rows, to check edges and the small sizes.
    w = sum(s + 16 for s in SIZES)
    prev = Image.new("RGBA", (w, 2 * (max(SIZES) + 16)), "#F3F3F3")
    prev.paste("#202020", (0, max(SIZES) + 16, w, prev.height))
    x = 8
    for s in SIZES:
        for row in range(2):
            prev.alpha_composite(images[s], (x, 8 + row * (max(SIZES) + 16)))
        x += s + 16
    (ROOT / "target").mkdir(exist_ok=True)
    prev.save(ROOT / "target" / "icon-preview.png")
    # Icon id 1: Explorer shows an exe's lowest-numbered icon, and the input
    # profile registers icon index 0 of the DLL.
    rc = sorted(glob.glob(r"C:\Program Files (x86)\Windows Kits\10\bin\*\x64\rc.exe"))[-1]
    (ASSETS / "app.rc").write_text('1 ICON "app.ico"\n', encoding="ascii")
    subprocess.run([rc, "/nologo", "/fo", "app.res", "app.rc"], cwd=ASSETS, check=True)
    (ASSETS / "app.rc").unlink()


if __name__ == "__main__":
    main()
