"""Generates assets/fpscraft.ico used by the installer and shortcuts.

Run from the repository root: python tools/make_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

SCALE = 4
SIZE = 256
CANVAS = SIZE * SCALE

BG = (10, 12, 17, 255)
ACCENT = (56, 189, 248, 255)
GLYPH = (236, 239, 244, 255)


def draw_icon() -> Image.Image:
    image = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    inset = 8 * SCALE
    draw.rounded_rectangle(
        (inset, inset, CANVAS - inset, CANVAS - inset),
        radius=48 * SCALE,
        fill=BG,
        outline=ACCENT,
        width=10 * SCALE,
    )

    draw.rounded_rectangle((72 * SCALE, 64 * SCALE, 104 * SCALE, 192 * SCALE), radius=4 * SCALE, fill=GLYPH)
    draw.rounded_rectangle((72 * SCALE, 64 * SCALE, 184 * SCALE, 92 * SCALE), radius=4 * SCALE, fill=GLYPH)
    draw.rounded_rectangle((72 * SCALE, 116 * SCALE, 160 * SCALE, 144 * SCALE), radius=4 * SCALE, fill=GLYPH)

    return image.resize((SIZE, SIZE), Image.LANCZOS)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    assets = root / "assets"
    assets.mkdir(exist_ok=True)

    icon = draw_icon()
    icon.save(
        assets / "fpscraft.ico",
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    icon.save(assets / "fpscraft.png", format="PNG")
    print(f"wrote {assets / 'fpscraft.ico'}")


if __name__ == "__main__":
    main()
