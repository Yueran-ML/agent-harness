SELECT c.name, COUNT(*) AS n FROM orders o JOIN customers c ON c.id = o.customer_id GROUP BY c.id HAVING n > 4 ORDER BY n DESC, c.name
