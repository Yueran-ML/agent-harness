SELECT c.name, CAST(julianday(MIN(o.ordered)) - julianday(c.joined) AS INTEGER) AS days FROM customers c JOIN orders o ON o.customer_id = c.id GROUP BY c.id ORDER BY days DESC, c.name
