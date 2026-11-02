# Compares the first channel of two 8-bit PNGs texel by texel, with no image library: how many texels differ, the
# largest difference, and how many differ by more than eight levels. Use it on intermediate render targets, where
# an optimization that claims to be exact must produce zero differing texels.
#
# usage: pngdiff.py before.png after.png
import struct
import sys
import zlib


def decode(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    position, chunks, width, height, color, depth = 8, [], 0, 0, 0, 0
    while position < len(data):
        length, kind = struct.unpack(">I4s", data[position : position + 8])
        body = data[position + 8 : position + 8 + length]
        position += 12 + length
        if kind == b"IHDR":
            width, height, depth, color = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            chunks.append(body)
    assert depth == 8, (path, depth)
    channels = {0: 1, 2: 3, 4: 2, 6: 4}[color]
    raw = zlib.decompress(b"".join(chunks))
    stride = width * channels
    rows, previous, offset = [], bytearray(stride), 0
    for _ in range(height):
        filter_type = raw[offset]
        line = bytearray(raw[offset + 1 : offset + 1 + stride])
        offset += 1 + stride
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = previous[i]
            c = previous[i - channels] if i >= channels else 0
            if filter_type == 1:
                line[i] = (line[i] + a) & 255
            elif filter_type == 2:
                line[i] = (line[i] + b) & 255
            elif filter_type == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif filter_type == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        previous = line
    return width, height, channels, rows


before, after = sys.argv[1], sys.argv[2]
width, height, channels, rows_a = decode(before)
shape_b = decode(after)
assert (width, height, channels) == shape_b[:3], ((width, height, channels), shape_b[:3])
differing = large = worst = 0
for row_a, row_b in zip(rows_a, shape_b[3]):
    for x in range(0, width * channels, channels):
        delta = abs(row_a[x] - row_b[x])
        if delta:
            differing += 1
            worst = max(worst, delta)
            large += delta > 8
total = width * height
print(
    f"{before.rsplit('/', 1)[-1]} vs {after.rsplit('/', 1)[-1]}: {width}x{height}x{channels} "
    f"differing={differing} ({100 * differing / total:.3f}%) max={worst} over8={large}"
)
