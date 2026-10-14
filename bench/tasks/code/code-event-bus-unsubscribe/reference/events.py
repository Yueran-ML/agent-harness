"""A small bus of events: handlers subscribe to an event, and are called when it is published."""

from collections import defaultdict


class EventBus:
    """Calls the handlers of an event, in the order in which they subscribed, when it is published."""

    def __init__(self):
        self._handlers = defaultdict(list)

    def subscribe(self, event, handler):
        """Call ``handler(payload)`` each time ``event`` is published.

        Returns:
            A function of no arguments that unsubscribes the handler.
        """
        self._handlers[event].append(handler)
        return lambda: self.unsubscribe(event, handler)

    def once(self, event, handler):
        """Call ``handler(payload)`` the next time ``event`` is published, and not after that.

        Returns:
            A function of no arguments that unsubscribes the handler.
        """

        def call_once(payload):
            self.unsubscribe(event, call_once)
            handler(payload)

        return self.subscribe(event, call_once)

    def unsubscribe(self, event, handler):
        """Stop calling ``handler`` for ``event``. A handler that is not subscribed is ignored."""
        handlers = self._handlers.get(event)
        if handlers and handler in handlers:
            handlers.remove(handler)

    def publish(self, event, payload=None):
        """Call each handler of ``event`` with ``payload``, and return how many were called."""
        called = 0
        for handler in list(self._handlers.get(event, [])):
            handler(payload)
            called += 1
        return called

    def handlers(self, event):
        """Return the handlers that are subscribed to ``event``, in order."""
        return list(self._handlers.get(event, []))
