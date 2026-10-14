import unittest

from events import EventBus


class Log:
    """Handlers that write down that they were called, and with what."""

    def __init__(self):
        self.calls = []

    def handler(self, name):
        def handle(payload):
            self.calls.append((name, payload))

        return handle


class ChangesDuringPublishTest(unittest.TestCase):
    def test_once_handler_followed_by_others(self):
        bus, log = EventBus(), Log()
        bus.once("saved", log.handler("once"))
        bus.subscribe("saved", log.handler("a"))
        bus.subscribe("saved", log.handler("b"))
        self.assertEqual(bus.publish("saved", 1), 3)
        self.assertEqual(bus.publish("saved", 2), 2)
        self.assertEqual(log.calls, [("once", 1), ("a", 1), ("b", 1), ("a", 2), ("b", 2)])

    def test_two_once_handlers_in_a_row(self):
        bus, log = EventBus(), Log()
        bus.once("saved", log.handler("first"))
        bus.once("saved", log.handler("second"))
        bus.subscribe("saved", log.handler("always"))
        self.assertEqual(bus.publish("saved", 1), 3)
        self.assertEqual(bus.publish("saved", 2), 1)
        self.assertEqual(log.calls, [("first", 1), ("second", 1), ("always", 1), ("always", 2)])

    def test_handler_that_unsubscribes_itself(self):
        bus, log = EventBus(), Log()
        bus.subscribe("tick", log.handler("a"))
        record_b = log.handler("b")

        def b_then_leave(payload):
            record_b(payload)
            leave()

        leave = bus.subscribe("tick", b_then_leave)
        bus.subscribe("tick", log.handler("c"))
        bus.subscribe("tick", log.handler("d"))
        bus.publish("tick", 1)
        bus.publish("tick", 2)
        self.assertEqual(
            log.calls,
            [("a", 1), ("b", 1), ("c", 1), ("d", 1), ("a", 2), ("c", 2), ("d", 2)],
        )

    def test_unsubscribing_an_earlier_handler(self):
        bus, log = EventBus(), Log()
        first = log.handler("first")
        bus.subscribe("tick", first)

        def remove_first(payload):
            log.calls.append(("remover", payload))
            bus.unsubscribe("tick", first)

        bus.subscribe("tick", remove_first)
        bus.subscribe("tick", log.handler("last"))
        bus.publish("tick", 1)
        bus.publish("tick", 2)
        self.assertEqual(
            log.calls,
            [("first", 1), ("remover", 1), ("last", 1), ("remover", 2), ("last", 2)],
        )

    def test_unsubscribing_a_later_handler_takes_effect_next_time(self):
        bus, log = EventBus(), Log()
        later = log.handler("later")

        def remove_later(payload):
            log.calls.append(("remover", payload))
            bus.unsubscribe("tick", later)

        bus.subscribe("tick", remove_later)
        bus.subscribe("tick", log.handler("middle"))
        bus.subscribe("tick", later)
        self.assertEqual(bus.publish("tick", 1), 3)
        self.assertEqual(bus.publish("tick", 2), 2)
        self.assertEqual(
            log.calls,
            [("remover", 1), ("middle", 1), ("later", 1), ("remover", 2), ("middle", 2)],
        )

    def test_subscribing_during_publish_takes_effect_next_time(self):
        bus, log = EventBus(), Log()
        added = []

        def add_one(payload):
            log.calls.append(("adder", payload))
            if not added:
                added.append(bus.subscribe("tick", log.handler("new")))

        bus.subscribe("tick", add_one)
        bus.subscribe("tick", log.handler("old"))
        self.assertEqual(bus.publish("tick", 1), 2)
        self.assertEqual(bus.publish("tick", 2), 3)
        self.assertEqual(
            log.calls,
            [("adder", 1), ("old", 1), ("adder", 2), ("old", 2), ("new", 2)],
        )

    def test_publishing_from_a_handler(self):
        bus, log = EventBus(), Log()
        bus.once("order.paid", lambda payload: bus.publish("receipt", payload))
        bus.subscribe("order.paid", log.handler("ledger"))
        bus.subscribe("receipt", log.handler("mailer"))
        bus.publish("order.paid", 42)
        bus.publish("order.paid", 43)
        self.assertEqual(log.calls, [("mailer", 42), ("ledger", 42), ("ledger", 43)])


class PublishTest(unittest.TestCase):
    def test_handlers_in_the_order_they_subscribed(self):
        bus, log = EventBus(), Log()
        for name in "abc":
            bus.subscribe("tick", log.handler(name))
        self.assertEqual(bus.publish("tick", 7), 3)
        self.assertEqual(log.calls, [("a", 7), ("b", 7), ("c", 7)])

    def test_unsubscribe_function(self):
        bus, log = EventBus(), Log()
        unsubscribe = bus.subscribe("tick", log.handler("a"))
        bus.subscribe("tick", log.handler("b"))
        unsubscribe()
        unsubscribe()
        self.assertEqual(bus.publish("tick"), 1)
        self.assertEqual(log.calls, [("b", None)])

    def test_events_are_kept_apart(self):
        bus, log = EventBus(), Log()
        bus.once("saved", log.handler("once"))
        bus.subscribe("deleted", log.handler("deleted"))
        self.assertEqual(bus.publish("deleted", 1), 1)
        self.assertEqual(bus.publish("saved", 2), 1)
        self.assertEqual(bus.publish("saved", 3), 0)
        self.assertEqual(bus.publish("nothing", 4), 0)
        self.assertEqual(log.calls, [("deleted", 1), ("once", 2)])


if __name__ == "__main__":
    unittest.main()
