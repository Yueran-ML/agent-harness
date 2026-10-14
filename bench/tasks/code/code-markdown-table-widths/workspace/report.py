"""The sales of each region, as a Markdown table."""

from mdtable import render_table


def sales_table(sales):
    """Return a table of the units sold and the revenue of each region, the largest revenue first.

    Args:
        sales: A dict of the name of each region to its units sold and its revenue.
    """
    ordered = sorted(sales.items(), key=lambda item: (-item[1][1], item[0]))
    rows = [[region, units, f"{revenue:,.2f}"] for region, (units, revenue) in ordered]
    return render_table(["Region", "Units", "Revenue"], rows, align=[None, "right", "right"])
