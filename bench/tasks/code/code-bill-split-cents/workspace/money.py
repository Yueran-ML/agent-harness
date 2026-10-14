"""Amounts of money, kept as whole numbers of cents."""


def to_cents(amount: str) -> int:
    """Return the number of cents in an amount written like "12.34", "12.3", "12" or "$12.34"."""
    text = amount.strip().removeprefix("$")
    value = float(text)
    if value < 0:
        raise ValueError(f"an amount cannot be negative: {amount!r}")
    return int(value * 100)


def format_cents(cents: int) -> str:
    """Write a number of cents as an amount: 1234 gives "12.34"."""
    sign = "-" if cents < 0 else ""
    whole, rest = divmod(abs(cents), 100)
    return f"{sign}{whole}.{rest:02d}"
