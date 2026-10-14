import unittest

from lru import LRUCache, lru_cached


class ModelLRU:
    """A plain list-based LRU cache to check the real one against."""

    def __init__(self, capacity):
        self.capacity = capacity
        self.order = []
        self.values = {}

    def get(self, key, default=None):
        if key not in self.values:
            return default
        self.order.remove(key)
        self.order.append(key)
        return self.values[key]

    def put(self, key, value):
        if key in self.values:
            self.order.remove(key)
        self.order.append(key)
        self.values[key] = value
        if len(self.order) > self.capacity:
            del self.values[self.order.pop(0)]


class EvictionTest(unittest.TestCase):
    def test_capacity_one_keeps_the_newest_entry(self):
        cache = LRUCache(1)
        cache.put("x", 1)
        cache.put("y", 2)
        self.assertEqual(len(cache), 1)
        self.assertEqual(cache.get("y"), 2)
        self.assertNotIn("x", cache)

    def test_without_reads_the_oldest_entries_go_first(self):
        cache = LRUCache(3)
        for number in range(1, 6):
            cache.put(number, str(number))
        self.assertEqual(cache.keys(), [3, 4, 5])

    def test_reading_an_entry_keeps_it(self):
        cache = LRUCache(3)
        for key in "abc":
            cache.put(key, key.upper())
        self.assertEqual(cache.get("a"), "A")
        cache.put("d", "D")
        self.assertEqual(cache.keys(), ["c", "a", "d"])
        self.assertNotIn("b", cache)

    def test_writing_an_existing_entry_keeps_it_and_replaces_its_value(self):
        cache = LRUCache(2)
        cache.put("a", 1)
        cache.put("b", 2)
        cache.put("a", 10)
        cache.put("c", 3)
        self.assertEqual(cache.keys(), ["a", "c"])
        self.assertEqual(cache.get("a"), 10)

    def test_membership_and_misses_do_not_count_as_use(self):
        cache = LRUCache(2)
        cache.put("a", 1)
        cache.put("b", 2)
        self.assertIn("a", cache)
        self.assertIsNone(cache.get("missing"))
        cache.put("c", 3)
        self.assertEqual(cache.keys(), ["b", "c"])

    def test_long_run_matches_a_model(self):
        for capacity in (1, 2, 3, 5):
            cache = LRUCache(capacity)
            model = ModelLRU(capacity)
            for step in range(300):
                key = (step * 7 + step // 5) % 9
                if step % 3 == 0:
                    self.assertEqual(cache.get(key), model.get(key), (capacity, step))
                else:
                    cache.put(key, step)
                    model.put(key, step)
                self.assertEqual(cache.keys(), model.order, (capacity, step))


class DecoratorTest(unittest.TestCase):
    def test_decorated_function_forgets_the_least_recently_used_arguments(self):
        computed = []

        @lru_cached(2)
        def double(n):
            computed.append(n)
            return 2 * n

        results = [double(n) for n in (1, 2, 1, 3, 1, 2)]
        self.assertEqual(results, [2, 4, 2, 6, 2, 4])
        self.assertEqual(computed, [1, 2, 3, 2])
        self.assertEqual(double.cache.keys(), [(1,), (2,)])


if __name__ == "__main__":
    unittest.main()
