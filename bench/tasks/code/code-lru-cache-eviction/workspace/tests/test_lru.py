import unittest

from lru import LRUCache, lru_cached


class LRUCacheTest(unittest.TestCase):
    def test_get_returns_what_was_put(self):
        cache = LRUCache(2)
        cache.put("a", 1)
        self.assertEqual(cache.get("a"), 1)
        self.assertIsNone(cache.get("b"))
        self.assertEqual((cache.hits, cache.misses), (1, 1))

    def test_full_cache_drops_the_least_recently_used_entry(self):
        cache = LRUCache(2)
        cache.put("a", 1)
        cache.put("b", 2)
        cache.get("a")
        cache.put("c", 3)
        self.assertEqual(cache.get("c"), 3)
        self.assertEqual(cache.get("a"), 1)
        self.assertIsNone(cache.get("b"))

    def test_decorator_remembers_results(self):
        calls = []

        @lru_cached(4)
        def square(n):
            calls.append(n)
            return n * n

        self.assertEqual([square(3), square(3), square(4)], [9, 9, 16])
        self.assertEqual(calls, [3, 4])


if __name__ == "__main__":
    unittest.main()
