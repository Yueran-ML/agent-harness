SELECT ROUND(SUM(i.quantity * p.price), 2) FROM order_items i JOIN orders o ON o.id = i.order_id JOIN products p ON p.id = i.product_id WHERE o.status = 'delivered'
