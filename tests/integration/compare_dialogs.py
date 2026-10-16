"""Side-by-side, 50% overlay and difference images of an XDM dialog against a reference crop.

usage: compare_dialogs.py OURS.png x,y,w,h REF.png x,y,w,h OUT_PREFIX [scale]
Both crops are registered on their top-left corner; the reference is local analysis data
and is never copied into the repository.
"""
import sys

from PIL import Image, ImageChops


def crop(path, rect):
    x, y, w, h = map(int, rect.split(","))
    return Image.open(path).convert("RGB").crop((x, y, x + w, y + h))


def main():
    ours_path, ours_rect, ref_path, ref_rect, prefix = sys.argv[1:6]
    scale = int(sys.argv[6]) if len(sys.argv) > 6 else 2
    ours, ref = crop(ours_path, ours_rect), crop(ref_path, ref_rect)
    width, height = max(ours.width, ref.width), max(ours.height, ref.height)
    canvas = lambda image: (lambda c: (c.paste(image, (0, 0)), c)[1])(Image.new("RGB", (width, height), (255, 0, 255)))
    ours_c, ref_c = canvas(ours), canvas(ref)
    side = Image.new("RGB", (width * 2 + 8, height), (0, 0, 0))
    side.paste(ref_c, (0, 0))
    side.paste(ours_c, (width + 8, 0))
    overlay = Image.blend(ref_c, ours_c, 0.5)
    diff = ImageChops.difference(ref_c, ours_c)
    for name, image in (("side", side), ("overlay", overlay), ("diff", diff)):
        image.resize((image.width * scale, image.height * scale), Image.NEAREST).save(f"{prefix}-{name}.png")
    print(f"ours {ours.size} reference {ref.size}")


main()
