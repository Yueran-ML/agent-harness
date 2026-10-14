SELECT category, name, price FROM products p WHERE price = (SELECT MAX(price) FROM products q WHERE q.category = p.category)
