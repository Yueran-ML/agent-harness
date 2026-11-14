"""Reference frames of colour management, for the tests of src/render/managed.rs.

Run with the Python of a venv that has `opencolorio` of the version the engine links (2.6.0):

    python -I make.py

For each space of the reference (the first that the author asked for, Q4 of the plan: the HLG
and PQ of phones, Apple Log and DJI D-Log; and S-Log3 and LogC4), known values of linear light
(of Rec.2020, 1.0 at 100 nits) are encoded into the space by OCIO's processor, made Y'CbCr of
BT.2020 (non-constant luminance) of 10 bits of limited range, rounded to codes, and written to
reference.json with what the engine is to make of them in a timeline of DaVinci Wide Gamut /
DaVinci Intermediate: `expected`, the codes decoded as they are (by the matrix and range, not
clamped) and taken into the timeline by OCIO; and `ideal`, the values of linear light taken into
the timeline as they were, before they were made codes of 10 bits.

The Rust test writes the codes as a picture of 4:4:4 (a patch of 8 by 8 pixels each), has the
ffmpeg of the command line make it a lossless FFV1 in Matroska tagged BT.2020 with the space's
transfer, reads it in as media, and draws it through the engine's input transform.
"""

import json
from pathlib import Path

import PyOpenColorIO

HERE = Path(__file__).resolve().parent

LINEAR = "Linear Rec.2020"
TIMELINE = "DaVinci Intermediate WideGamut"
KR, KB = 0.2627, 0.0593

# (the name of the catalogue, OCIO's space, the transfer that ffmpeg tags, the most of the space
# in linear light (1.0 at 100 nits) that the patches go to)
SPACES = [
    ("Rec.2100 HLG", "Rec.2100-HLG - Display", "arib-std-b67", 10.0),
    ("Rec.2100 ST2084", "Rec.2100-PQ - Display", "smpte2084", 40.0),
    ("Apple Log", "Apple Log", "unknown", 16.0),
    ("DJI D-Gamut/D-Log", "D-Log D-Gamut", "unknown", 16.0),
    ("Sony S-Gamut3.Cine/S-Log3", "S-Log3 S-Gamut3.Cine", "unknown", 16.0),
    ("ARRI Wide Gamut 4/LogC4", "ARRI LogC4", "unknown", 16.0),
]

PATCHES = [
    [0.0, 0.0, 0.0],
    [0.002, 0.002, 0.002],
    [0.01, 0.01, 0.01],
    [0.05, 0.05, 0.05],
    [0.18, 0.18, 0.18],
    [0.5, 0.5, 0.5],
    [1.0, 1.0, 1.0],
    [2.0, 2.0, 2.0],
    [0.3, 0.12, 0.05],
    [0.05, 0.2, 0.08],
    [0.04, 0.06, 0.5],
    [0.8, 0.6, 0.1],
    [1.5, 0.3, 0.2],
]


def apply(cpu, colours):
    """Apply the configured CPU transform to each RGB sample."""
    return [list(cpu.applyRGB([float(v) for v in c])) for c in colours]


def ycbcr(rgb):
    """Encode RGB as limited-range 10-bit BT.2020 code values."""
    r, g, b = rgb
    y = KR * r + (1 - KR - KB) * g + KB * b
    cb = (b - y) / (2 * (1 - KB))
    cr = (r - y) / (2 * (1 - KR))
    codes = [64 + 876 * y, 512 + 896 * cb, 512 + 896 * cr]
    return [min(1019, max(4, round(c))) for c in codes]


def decoded(codes):
    """Decode limited-range BT.2020 code values without clipping RGB."""
    y = (codes[0] - 64) / 876
    cb = (codes[1] - 512) / 896
    cr = (codes[2] - 512) / 896
    kg = 1 - KR - KB
    r = y + 2 * (1 - KR) * cr
    b = y + 2 * (1 - KB) * cb
    g = y - 2 * KB * (1 - KB) / kg * cb - 2 * KR * (1 - KR) / kg * cr
    return [r, g, b]


def main():
    """Write the fixed colour reference samples beside this generator."""
    config = PyOpenColorIO.Config.CreateFromFile("ocio://studio-config-latest")
    spaces = []
    for name, space, transfer, most in SPACES:
        patches = [c for c in PATCHES if max(c) <= most]
        encode = config.getProcessor(LINEAR, space).getDefaultCPUProcessor()
        into = config.getProcessor(space, TIMELINE).getDefaultCPUProcessor()
        ideal = config.getProcessor(LINEAR, TIMELINE).getDefaultCPUProcessor()
        codes = [ycbcr(rgb) for rgb in apply(encode, patches)]
        spaces.append(
            {
                "name": name,
                "ocio": space,
                "transfer": transfer,
                "linear": patches,
                "codes": codes,
                "expected": apply(into, [decoded(c) for c in codes]),
                "ideal": apply(ideal, patches),
            }
        )
    reference = {
        "ocio": PyOpenColorIO.__version__,
        "timeline": TIMELINE,
        "spaces": spaces,
    }
    with (HERE / "reference.json").open("w", encoding="utf-8", newline="\n") as out:
        json.dump(reference, out, indent=1)
        out.write("\n")


if __name__ == "__main__":
    main()
