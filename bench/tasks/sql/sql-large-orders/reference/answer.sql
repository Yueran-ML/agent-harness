SELECT order_id FROM order_items GROUP BY order_id HAVING COUNT(*) >= 4 ORDER BY order_id
