SELECT ROUND(100.0 * SUM(status = 'cancelled') / COUNT(*), 1) FROM orders
