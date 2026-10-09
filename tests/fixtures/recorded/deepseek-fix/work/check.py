"""Check the calculator. Prints ok, or the first thing that is wrong."""

import sys

from calc import mean

CASES = [([1.0, 2.0, 3.0], 2.0), ([2.0, 4.0], 3.0), ([5.0], 5.0)]


def main() -> int:
    """Run the cases and return 0 if all of them hold."""
    for values, expected in CASES:
        try:
            got = mean(values)
        except ArithmeticError as error:
            print(f"mean({values}) raised {type(error).__name__}, expected {expected}")
            return 1
        if got != expected:
            print(f"mean({values}) is {got}, expected {expected}")
            return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
