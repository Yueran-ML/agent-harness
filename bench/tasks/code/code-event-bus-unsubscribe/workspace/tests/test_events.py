import unittest

from events import EventBus


class EventBusTest(unittest.TestCase):
    def test_handlers_are_called_in_order(self):
        bus = EventBus()
        calls = []
        bus.subscribe("saved", lambda payload: calls.append(("first", payload)))
        bus.subscribe("saved", lambda payload: calls.append(("second", payload)))
        self.assertEqual(bus.publish("saved", 1), 2)
        self.assertEqual(calls, [("first", 1), ("second", 1)])

    def test_unsubscribe(self):
        bus = EventBus()
        calls = []
        unsubscribe = bus.subscribe("saved", calls.append)
        unsubscribe()
        self.assertEqual(bus.publish("saved", 1), 0)
        self.assertEqual(calls, [])

    def test_once_does_not_skip_the_next_handler(self):
        bus = EventBus()
        calls = []
        bus.once("saved", lambda payload: calls.append(("once", payload)))
        bus.subscribe("saved", lambda payload: calls.append(("always", payload)))
        bus.publish("saved", 1)
        bus.publish("saved", 2)
        self.assertEqual(calls, [("once", 1), ("always", 1), ("always", 2)])


if __name__ == "__main__":
    unittest.main()
