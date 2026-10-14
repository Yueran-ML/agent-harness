SELECT substr(ordered, 1, 7) AS month, COUNT(*) AS n FROM orders GROUP BY month ORDER BY n DESC, month LIMIT 1
