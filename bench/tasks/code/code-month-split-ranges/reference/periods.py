"""Splitting a rental period into calendar months, so that each month is billed on its own."""

from datetime import date, timedelta

from months import days_in_month, month_end


def split_by_month(start: date, end: date) -> list[tuple[date, date]]:
    """Split the days from `start` to `end`, both included, into calendar months.

    Each piece is a pair (first day, last day), both included, and the pieces
    follow each other without a gap or an overlap. For example, 2024-01-30 to
    2024-02-02 gives [(2024-01-30, 2024-01-31), (2024-02-01, 2024-02-02)].
    """
    if end < start:
        raise ValueError("a period cannot end before it starts")
    pieces = []
    current = start
    while current <= end:
        last = min(month_end(current), end)
        pieces.append((current, last))
        current = last + timedelta(days=1)
    return pieces


def days_by_month(start: date, end: date) -> dict[tuple[int, int], int]:
    """Count the days of a period in each month, keyed by (year, month)."""
    return {
        (first.year, first.month): (last - first).days + 1
        for first, last in split_by_month(start, end)
    }


def rent_by_month(start: date, end: date, monthly_rent: int) -> dict[tuple[int, int], int]:
    """Charge each month of a period for the share of its days that are rented.

    `monthly_rent` is in cents, and so are the charges, which are rounded down.
    """
    return {
        (year, month): monthly_rent * days // days_in_month(year, month)
        for (year, month), days in days_by_month(start, end).items()
    }
