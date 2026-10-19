"""Assemble folder.ico à partir des rendus PNG de preview.html (icon-<taille>.png).

Usage : python make_ico.py DOSSIER_DES_PNG ../folder.ico

Les PNG sont produits en ouvrant preview.html dans un navigateur (servi en http local, pour
que le canevas ne soit pas « teinté ») : window.pngs contient chaque taille en base64.
Entrées de l'.ico stockées en PNG (accepté par Windows depuis Vista), de la plus grande à la
plus petite.
"""
import struct
import sys
from pathlib import Path

SIZES = [256, 128, 96, 64, 48, 40, 32, 24, 20, 16]


def main(src: Path, out: Path) -> None:
    blobs = [(s, (src / f'icon-{s}.png').read_bytes()) for s in SIZES]
    header = struct.pack('<HHH', 0, 1, len(blobs))
    offset = len(header) + 16 * len(blobs)
    entries, data = b'', b''
    for size, png in blobs:
        dim = 0 if size >= 256 else size
        entries += struct.pack('<BBBBHHII', dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    out.write_bytes(header + entries + data)
    print(f'{out} : {len(blobs)} tailles, {out.stat().st_size} octets')


if __name__ == '__main__':
    main(Path(sys.argv[1]), Path(sys.argv[2]))
