"""Reference values of OpenColorIO's Python bindings (PyOpenColorIO), for tests/reference.rs.

Run with the Python of a venv that has `opencolorio` of the version the crate links (2.6.0):

    python -I make.py

It reads spaces.txt (the OCIO names of the catalogue, one a line, which the Rust test holds to
catalogue::spaces) and ../../config/columbina.ocio (our fragment), adds the fragment's colour
spaces to the built-in studio config as Config::columbina does, and writes reference.json: for
each space, fixed inputs to ACES2065-1, from ACES2065-1, and to the ACES 2.0 SDR view of a
Rec.709 display, through OCIO's CPU processor (32-bit floats, default optimization).
"""

import json
import os

import PyOpenColorIO as ocio

HERE = os.path.dirname(os.path.abspath(__file__))

ENCODED = [
    [0.0, 0.0, 0.0], [0.1, 0.1, 0.1], [0.18, 0.18, 0.18], [0.4, 0.4, 0.4], [0.6, 0.6, 0.6],
    [0.9, 0.9, 0.9], [1.0, 1.0, 1.0], [0.7, 0.3, 0.2], [0.2, 0.6, 0.3], [0.25, 0.3, 0.8],
    [0.95, 0.5, 0.05], [0.05, 0.02, 0.01],
]
LINEAR = [
    [0.0, 0.0, 0.0], [0.001, 0.001, 0.001], [0.01, 0.01, 0.01], [0.18, 0.18, 0.18],
    [0.5, 0.5, 0.5], [1.0, 1.0, 1.0], [4.0, 4.0, 4.0], [16.0, 16.0, 16.0], [0.3, 0.12, 0.05],
    [0.05, 0.2, 0.08], [0.04, 0.06, 0.5], [2.0, 0.8, 0.2],
]
DISPLAY = "Rec.1886 Rec.709 - Display"
VIEW = "ACES 2.0 - SDR 100 nits (Rec.709)"


def apply(processor, colours):
    cpu = processor.getDefaultCPUProcessor()
    return [list(cpu.applyRGB(list(map(float, c)))) for c in colours]


def main():
    config = ocio.Config.CreateFromFile("ocio://studio-config-latest")
    fragment = ocio.Config.CreateFromFile(os.path.join(HERE, "..", "..", "config", "columbina.ocio"))
    for space in fragment.getColorSpaces(ocio.SEARCH_REFERENCE_SPACE_ALL, ocio.COLORSPACE_ALL):
        config.addColorSpace(space)
    config.validate()
    names = [line.strip() for line in open(os.path.join(HERE, "spaces.txt"), encoding="utf-8") if line.strip()]
    out = {"ocio": ocio.__version__, "config": config.getName(), "spaces": {}}
    for name in names:
        space = config.getColorSpace(name)
        linear = space.getEncoding() in ("scene-linear", "display-linear")
        inputs = LINEAR if linear else ENCODED
        to_aces = config.getProcessor(name, "ACES2065-1")
        from_aces = config.getProcessor("ACES2065-1", name)
        display = ocio.DisplayViewTransform(src=name, display=DISPLAY, view=VIEW)
        out["spaces"][name] = {
            "inputs": inputs,
            "to_aces": apply(to_aces, inputs),
            "from_aces": apply(from_aces, LINEAR),
            "to_display": apply(config.getProcessor(display), inputs),
        }
    out["aces_inputs"] = LINEAR
    with open(os.path.join(HERE, "reference.json"), "w", encoding="utf-8") as f:
        json.dump(out, f)


if __name__ == "__main__":
    main()
