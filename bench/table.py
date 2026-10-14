"""The tables of results, from the lines that the attempts left, in Markdown.

A rate of success is given with its interval of 95 percent by the method of
Wilson, which stays between 0 and 1 and holds for small counts. The
difference of a variant from the whole program is given with the interval of
Newcombe, made from the two intervals of Wilson. Tokens are given as the
median of one attempt. Its input is all that it put in, whether the service
read it from its cache or not, since Columbina counts the two apart. The
share from the cache is the tokens read from it over all the tokens put in,
of all the attempts of a row together.

``matrix`` marks a model usable where it passes half of the attempts or more.
"""

import json
import math
import statistics
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any

__all__ = ["CATEGORIES", "USABLE", "acceptance", "matrix", "newcombe", "read", "variants", "wilson"]

Z = 1.959963984540054
"""How many standard deviations from the middle hold 95 percent of a normal distribution."""

CATEGORIES = ("code", "files", "sql", "compute")
"""The categories, in the order of the columns."""

USABLE = 0.5
"""The rate of success from which a model is marked usable."""


def wilson(passed: int, total: int) -> tuple[float, float]:
    """Return the interval of Wilson of a rate of success, at 95 percent."""
    if total == 0:
        return (0.0, 1.0)
    rate = passed / total
    middle = rate + Z * Z / (2 * total)
    spread = Z * math.sqrt(rate * (1 - rate) / total + Z * Z / (4 * total * total))
    scale = 1 + Z * Z / total
    return (max(0.0, (middle - spread) / scale), min(1.0, (middle + spread) / scale))


def newcombe(one: tuple[int, int], other: tuple[int, int]) -> tuple[float, float]:
    """Return the interval at 95 percent of the first rate less the second."""
    rate_one, rate_other = one[0] / one[1], other[0] / other[1]
    low_one, high_one = wilson(*one)
    low_other, high_other = wilson(*other)
    difference = rate_one - rate_other
    below = math.sqrt((rate_one - low_one) ** 2 + (high_other - rate_other) ** 2)
    above = math.sqrt((high_one - rate_one) ** 2 + (rate_other - low_other) ** 2)
    return (difference - below, difference + above)


def read(files: Iterable[Path]) -> list[dict[str, Any]]:
    """Read the lines of results of one or more runs."""
    return [
        json.loads(line)
        for file in files
        for line in file.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def _rate(lines: Sequence[dict[str, Any]]) -> str:
    """Write a rate of success as ``passed/total, percent [low, high]``."""
    passed, total = sum(bool(line["passed"]) for line in lines), len(lines)
    if not total:
        return "-"
    low, high = wilson(passed, total)
    return f"{passed}/{total} = {passed / total:.0%} [{low:.0%}, {high:.0%}]"


_PUT = ("input_tokens", "cache_read_tokens", "cache_write_tokens")
"""The counts of what goes in, which Columbina keeps apart: none of them holds another."""


def _put(line: dict[str, Any]) -> int:
    """Return all the tokens that an attempt put in, those that a cache held among them."""
    usage = line["usage"]
    return sum(usage.get(kind, 0) for kind in _PUT)


def _median(counts: Sequence[int]) -> int:
    return round(statistics.median(counts)) if counts else 0


def _tokens(lines: Sequence[dict[str, Any]]) -> str:
    """Write the tokens of one attempt, put in and given out, as two cells."""
    given_out = [line["usage"].get("output_tokens", 0) for line in lines]
    return f"{_median([_put(line) for line in lines])} | {_median(given_out)}"


def _cached(lines: Sequence[dict[str, Any]]) -> str:
    put = sum(_put(line) for line in lines)
    read_from_cache = sum(line["usage"].get("cache_read_tokens", 0) for line in lines)
    return f"{read_from_cache / put:.0%}" if put else "-"


def _of(lines: Sequence[dict[str, Any]], **fields: object) -> list[dict[str, Any]]:
    return [line for line in lines if all(line[key] == value for key, value in fields.items())]


def acceptance(lines: Sequence[dict[str, Any]]) -> str:
    """The table of the acceptance benchmark: each model, by category, the whole program on."""
    lines = _of(lines, variant="all")
    rows = [
        "| Model | "
        + " | ".join(CATEGORIES)
        + " | All | Input tokens | Output tokens | From cache |",
        "|---|" + "---|" * (len(CATEGORIES) + 4),
    ]
    for model in sorted({line["model"] for line in lines}):
        mine = _of(lines, model=model)
        cells = [_rate(_of(mine, category=category)) for category in CATEGORIES]
        rows.append(
            f"| {model} | "
            + " | ".join(cells)
            + f" | {_rate(mine)} | {_tokens(mine)} | {_cached(mine)} |"
        )
    return "\n".join(rows) + "\n"


def variants(lines: Sequence[dict[str, Any]], model: str) -> str:
    """The table of the parts of the program: each switched off in turn, against all of them on."""
    lines = _of(lines, model=model)
    whole = _of(lines, variant="all")
    passed = sum(bool(line["passed"]) for line in whole)
    rows = [
        "| Variant | All | Difference | Input tokens | Output tokens |",
        "|---|---|---|---|---|",
        f"| all | {_rate(whole)} | | {_tokens(whole)} |",
    ]
    for variant in sorted({line["variant"] for line in lines} - {"all"}):
        mine = _of(lines, variant=variant)
        mine_passed = sum(bool(line["passed"]) for line in mine)
        if whole and mine:
            low, high = newcombe((mine_passed, len(mine)), (passed, len(whole)))
            difference = (
                f"{mine_passed / len(mine) - passed / len(whole):+.0%} [{low:+.0%}, {high:+.0%}]"
            )
        else:
            difference = "-"
        rows.append(f"| {variant} | {_rate(mine)} | {difference} | {_tokens(mine)} |")
    return "\n".join(rows) + "\n"


def matrix(lines: Sequence[dict[str, Any]]) -> str:
    """The table of local models: each by category, and whether it is usable."""
    lines = _of(lines, variant="all")
    rows = [
        "| Model | " + " | ".join(CATEGORIES) + " | All | Usable |",
        "|---|" + "---|" * (len(CATEGORIES) + 2),
    ]
    for model in sorted({line["model"] for line in lines}):
        mine = _of(lines, model=model)
        rate = sum(bool(line["passed"]) for line in mine) / len(mine)
        cells = [_rate(_of(mine, category=category)) for category in CATEGORIES]
        rows.append(
            f"| {model} | "
            + " | ".join(cells)
            + f" | {_rate(mine)} | {'yes' if rate >= USABLE else 'no'} |"
        )
    return "\n".join(rows) + "\n"
