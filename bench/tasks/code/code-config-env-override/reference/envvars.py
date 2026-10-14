"""Settings that are given in environment variables."""

_TRUE = frozenset({"1", "true", "yes", "on"})
_FALSE = frozenset({"0", "false", "no", "off"})


def read_overrides(environ, prefix):
    """Return the settings that the variables whose names begin with ``prefix`` give.

    The rest of the name of such a variable, in lower case, is the name of the
    setting: with the prefix ``SHOP_``, ``SHOP_LOG_LEVEL=debug`` gives
    ``{"log_level": "debug"}``. The values are the text of the variables.
    """
    found = {}
    for variable, text in environ.items():
        if variable.startswith(prefix):
            found[variable[len(prefix):].lower()] = text
    return found


def parse_value(text, default):
    """Return the text of a variable as a value of the type of a setting's default.

    A boolean is ``true``, ``yes``, ``on`` or ``1``, or ``false``, ``no``,
    ``off`` or ``0``, in any case.

    Raises:
        ValueError: The text is not a value of that type.
    """
    if isinstance(default, bool):
        word = text.strip().lower()
        if word in _TRUE:
            return True
        if word in _FALSE:
            return False
        raise ValueError(f"{text!r} is not a boolean")
    if isinstance(default, int):
        return int(text)
    if isinstance(default, float):
        return float(text)
    return text
