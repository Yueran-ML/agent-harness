"""The database of the tasks that ask a database, and the questions about it, made from a seed.

The database is a small shop: customers in some cities, products in some
categories, orders with a status, and the items of each order. It is made
twice, from two seeds: the copy that a model asks, and a changed copy that the
query it submits is run on as well. The two have the same tables and other
rows, so a query that holds the answer of the first returns the wrong rows on
the second.

The data is kept as text, a list of SQL statements, and made into a file of
SQLite for each attempt.
"""

import random
import sqlite3
from dataclasses import dataclass
from pathlib import Path

from bench.database import STATEMENTS
from bench.generate import files

__all__ = ["QUESTIONS", "Question", "dump", "written"]

SCHEMA = """CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    city TEXT NOT NULL,
    joined TEXT NOT NULL
);
CREATE TABLE products (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    price REAL NOT NULL
);
CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers (id),
    ordered TEXT NOT NULL,
    status TEXT NOT NULL
);
CREATE TABLE order_items (
    order_id INTEGER NOT NULL REFERENCES orders (id),
    product_id INTEGER NOT NULL REFERENCES products (id),
    quantity INTEGER NOT NULL,
    PRIMARY KEY (order_id, product_id)
);
"""

CITIES = ("Brisbane", "Sydney", "Melbourne", "Perth", "Adelaide", "Hobart")
FIRST = (
    "Ava",
    "Ben",
    "Chloe",
    "Dan",
    "Ella",
    "Finn",
    "Grace",
    "Hugo",
    "Isla",
    "Jack",
    "Kai",
    "Lily",
    "Mia",
    "Noah",
    "Olive",
    "Pip",
    "Quinn",
    "Ruby",
    "Sam",
    "Tess",
)
LAST = ("Ng", "Smith", "Chen", "Brown", "Wilson", "Taylor", "Lee", "Martin", "White", "Hall")
CATEGORIES = {
    "kitchen": ("kettle", "toaster", "pan", "knife set", "chopping board"),
    "garden": ("hose", "rake", "seed tray", "watering can", "pruner"),
    "office": ("stapler", "desk lamp", "notebook", "pen pack", "monitor stand"),
    "outdoor": ("tent", "lantern", "cooler", "camp chair", "sleeping bag"),
}
STATUSES = ("delivered", "delivered", "delivered", "shipped", "cancelled")


def dump(seed: int) -> str:
    """Return the statements that make the shop of a seed: the same seed, the same text."""
    rng = random.Random(seed)
    lines = [SCHEMA]
    names: set[str] = set()
    customers = rng.randint(24, 30)
    for number in range(1, customers + 1):
        while True:
            name = f"{rng.choice(FIRST)} {rng.choice(LAST)}"
            if name not in names:
                names.add(name)
                break
        joined = f"2024-{rng.randint(1, 12):02d}-{rng.randint(1, 28):02d}"
        city = rng.choice(CITIES)
        lines.append(f"INSERT INTO customers VALUES ({number}, '{name}', '{city}', '{joined}');")
    product = 0
    for category, items in CATEGORIES.items():
        for item in items:
            product += 1
            price = rng.randint(5, 250) + rng.choice((0, 0.5, 0.95))
            lines.append(
                f"INSERT INTO products VALUES ({product}, '{item}', '{category}', {price});"
            )
    orders = rng.randint(60, 80)
    # Some customers never order, and some products are never ordered.
    buyers = rng.sample(range(1, customers + 1), customers - rng.randint(2, 4))
    stocked = rng.sample(range(1, product + 1), product - rng.randint(2, 4))
    for number in range(1, orders + 1):
        ordered = f"2025-{rng.randint(1, 9):02d}-{rng.randint(1, 28):02d}"
        lines.append(
            f"INSERT INTO orders VALUES ({number}, {rng.choice(buyers)}, '{ordered}', "
            f"'{rng.choice(STATUSES)}');"
        )
        lines.extend(
            f"INSERT INTO order_items VALUES ({number}, {item}, {rng.randint(1, 4)});"
            for item in sorted(rng.sample(stocked, rng.randint(1, 4)))
        )
    return "\n".join(lines) + "\n"


@dataclass(frozen=True)
class Question:
    """A question about the shop, and a query that answers it.

    Attributes:
        id: The name of its task.
        text: The question, with the columns and the order of the rows that
            the answer has.
        sql: A query that answers it.
    """

    id: str
    text: str
    sql: str


QUESTIONS = (
    Question(
        "sql-customers-per-city",
        "How many customers live in each city? Return the city and the number of customers.",
        "SELECT city, COUNT(*) FROM customers GROUP BY city",
    ),
    Question(
        "sql-orders-per-status",
        "How many orders are there of each status? Return the status and the number of orders.",
        "SELECT status, COUNT(*) FROM orders GROUP BY status",
    ),
    Question(
        "sql-delivered-revenue",
        "What is the total revenue of the delivered orders, that is the sum of quantity times "
        "price over their items? Return one row with one column, rounded to 2 decimal places.",
        "SELECT ROUND(SUM(i.quantity * p.price), 2) FROM order_items i "
        "JOIN orders o ON o.id = i.order_id JOIN products p ON p.id = i.product_id "
        "WHERE o.status = 'delivered'",
    ),
    Question(
        "sql-top-products",
        "Which five products sold the most units in orders that were not cancelled? Return the "
        "product name and the units, most units first, and by name where units are equal.",
        "SELECT p.name, SUM(i.quantity) AS units FROM order_items i "
        "JOIN orders o ON o.id = i.order_id JOIN products p ON p.id = i.product_id "
        "WHERE o.status <> 'cancelled' GROUP BY p.id ORDER BY units DESC, p.name LIMIT 5",
    ),
    Question(
        "sql-never-ordered-customers",
        "Which customers have never placed an order? Return their names.",
        "SELECT name FROM customers WHERE id NOT IN (SELECT customer_id FROM orders)",
    ),
    Question(
        "sql-never-ordered-products",
        "Which products have never been ordered? Return the product names.",
        "SELECT name FROM products WHERE id NOT IN (SELECT product_id FROM order_items)",
    ),
    Question(
        "sql-busiest-month",
        "In which month of 2025 were the most orders placed? Return the month as YYYY-MM and the "
        "number of orders; if months tie, return the earliest of them only.",
        "SELECT substr(ordered, 1, 7) AS month, COUNT(*) AS n FROM orders "
        "GROUP BY month ORDER BY n DESC, month LIMIT 1",
    ),
    Question(
        "sql-category-customers",
        "For each product category, how many different customers bought something from it in an "
        "order that was not cancelled? Return the category and the number of customers.",
        "SELECT p.category, COUNT(DISTINCT o.customer_id) FROM order_items i "
        "JOIN orders o ON o.id = i.order_id JOIN products p ON p.id = i.product_id "
        "WHERE o.status <> 'cancelled' GROUP BY p.category",
    ),
    Question(
        "sql-frequent-customers",
        "Which customers placed more than four orders? Return the name and the number of orders, "
        "most orders first, and by name where the numbers are equal.",
        "SELECT c.name, COUNT(*) AS n FROM orders o JOIN customers c ON c.id = o.customer_id "
        "GROUP BY c.id HAVING n > 4 ORDER BY n DESC, c.name",
    ),
    Question(
        "sql-revenue-per-category",
        "What revenue did each category bring in the delivered orders from April to June 2025, "
        "both months included? Return the category and the revenue rounded to 2 decimal places, "
        "the highest revenue first.",
        "SELECT p.category, ROUND(SUM(i.quantity * p.price), 2) AS revenue FROM order_items i "
        "JOIN orders o ON o.id = i.order_id JOIN products p ON p.id = i.product_id "
        "WHERE o.status = 'delivered' AND o.ordered >= '2025-04-01' AND o.ordered < '2025-07-01' "
        "GROUP BY p.category ORDER BY revenue DESC",
    ),
    Question(
        "sql-priciest-per-category",
        "What is the most expensive product of each category? Return the category, the product "
        "name and its price; where products of one category share the highest price, return each.",
        "SELECT category, name, price FROM products p WHERE price = "
        "(SELECT MAX(price) FROM products q WHERE q.category = p.category)",
    ),
    Question(
        "sql-large-orders",
        "Which orders hold four or more different products? Return the order ids, smallest first.",
        "SELECT order_id FROM order_items GROUP BY order_id HAVING COUNT(*) >= 4 ORDER BY order_id",
    ),
    Question(
        "sql-cancelled-share",
        "What percentage of all orders were cancelled? Return one row with one number, rounded "
        "to 1 decimal place.",
        "SELECT ROUND(100.0 * SUM(status = 'cancelled') / COUNT(*), 1) FROM orders",
    ),
    Question(
        "sql-city-average-order",
        "What is the average value of a delivered order in each city? The value of an order is the "
        "sum of quantity times price over its items. Return the city and the average rounded to "
        "2 decimal places, by city name.",
        "SELECT c.city, ROUND(AVG(t.value), 2) FROM (SELECT o.id, o.customer_id, "
        "SUM(i.quantity * p.price) AS value FROM orders o JOIN order_items i ON i.order_id = o.id "
        "JOIN products p ON p.id = i.product_id WHERE o.status = 'delivered' GROUP BY o.id) t "
        "JOIN customers c ON c.id = t.customer_id GROUP BY c.city ORDER BY c.city",
    ),
    Question(
        "sql-first-order-gap",
        "For the customers who placed at least one order, how many days passed from the day they "
        "joined to their first order? Return the name and the number of days, the longest wait "
        "first, and by name where the days are equal.",
        "SELECT c.name, CAST(julianday(MIN(o.ordered)) - julianday(c.joined) AS INTEGER) AS days "
        "FROM customers c JOIN orders o ON o.customer_id = c.id GROUP BY c.id "
        "ORDER BY days DESC, c.name",
    ),
)
"""The questions, one task each."""

PROMPT = (
    "Answer this question about the database of a shop:\n\n{text}\n\n"
    "The database is reached only with the tools list_tables, run_query and submit_sql. When "
    "you have a query whose rows answer the question, submit it with submit_sql: the rows of "
    "the last query that is submitted are the answer.\n"
)

README = (
    "# This task\n\n"
    "Nothing in this directory is needed. The database is reached with the tools list_tables, "
    "run_query and submit_sql.\n"
)


def _rows(statements: str, sql: str) -> list[tuple[object, ...]]:
    connection = sqlite3.connect(":memory:")
    try:
        connection.executescript(statements)
        return [tuple(row) for row in connection.execute(sql).fetchall()]
    finally:
        connection.close()


def written(questions: tuple[Question, ...], root: Path, *, seed: int, changed: int) -> None:
    """Write a task for each question, with the shop of ``seed`` and the changed one of ``changed``.

    Raises:
        ValueError: The query of a question returns nothing on one of the two
            shops, or the same rows on both: the changed shop would then not
            tell a query that holds the answer from one that finds it.
    """
    shop, other = dump(seed), dump(changed)
    for question in questions:
        first, second = _rows(shop, question.sql), _rows(other, question.sql)
        if not first or not second or first == second:
            raise ValueError(f"the question {question.id} does not tell the two shops apart")
    for question in questions:
        directory = root / question.id
        task = {
            "id": question.id,
            "category": "sql",
            "prompt": PROMPT.format(text=question.text),
            "timeout_seconds": 300,
            "max_steps": 25,
        }
        files.task_yml(directory, task)
        files.text(directory / "workspace" / "README.md", README)
        for name, statements in zip(STATEMENTS, (shop, other), strict=True):
            files.text(directory / "private" / name, statements)
        files.text(directory / "reference" / "answer.sql", question.sql + "\n")
