"""``python -m bench``: see :mod:`bench.cli`."""

import sys

from bench.cli import main

sys.exit(main(sys.argv[1:]))
