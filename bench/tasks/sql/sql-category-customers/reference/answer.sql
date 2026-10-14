SELECT p.category, COUNT(DISTINCT o.customer_id) FROM order_items i JOIN orders o ON o.id = i.order_id JOIN products p ON p.id = i.product_id WHERE o.status <> 'cancelled' GROUP BY p.category
