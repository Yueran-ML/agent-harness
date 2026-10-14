"""Totals by category of an expense export, for the monthly report."""

import csv
import io
from decimal import Decimal


def totals_by_category(text: str) -> dict[str, Decimal]:
    """Add up the amounts of a CSV export, category by category.

    The first line that is not blank is the header. It names the columns,
    among which there must be a "category" and an "amount" column: they may
    come in any order and be written in any case, with or without spaces
    around them, and the other columns are ignored. Blank lines, whether
    empty or holding only spaces, are skipped wherever they are. Amounts are
    decimal numbers such as 12.50.
    """
    rows = (row for row in csv.reader(io.StringIO(text)) if "".join(row).strip())
    header = [name.strip().lower() for name in next(rows, [])]
    if "category" not in header or "amount" not in header:
        raise ValueError('the header must name a "category" and an "amount" column')
    category_column = header.index("category")
    amount_column = header.index("amount")
    totals: dict[str, Decimal] = {}
    for row in rows:
        category, amount = row[category_column], row[amount_column]
        totals[category] = totals.get(category, Decimal("0")) + Decimal(amount)
    return totals


def format_report(totals: dict[str, Decimal]) -> str:
    """Write one line per category, in the order of their names, and a last line with the sum."""
    width = max([len("Total"), *(len(name) for name in totals)])
    lines = [f"{name:<{width}}  {amount:>10.2f}" for name, amount in sorted(totals.items())]
    lines.append(f"{'Total':<{width}}  {sum(totals.values(), Decimal('0')):>10.2f}")
    return "\n".join(lines)


def report_file(path: str) -> str:
    """Read an export from a file and return its report."""
    with open(path, encoding="utf-8", newline="") as file:
        return format_report(totals_by_category(file.read()))
