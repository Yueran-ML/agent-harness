"""Calendar months, for the billing periods of a rental."""

from datetime import date, timedelta


def month_start(day: date) -> date:
    """Return the first day of the month that `day` is in."""
    return day.replace(day=1)


def next_month_start(day: date) -> date:
    """Return the first day of the month after the one that `day` is in."""
    return date(day.year, day.month + 1, 1)


def month_end(day: date) -> date:
    """Return the last day of the month that `day` is in."""
    return next_month_start(day) - timedelta(days=1)


def days_in_month(year: int, month: int) -> int:
    """Return the number of days in a month of a year."""
    return month_end(date(year, month, 1)).day
