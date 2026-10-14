"""Settings that are given in environment variables."""


def read_overrides(environ, prefix):
    """Return the settings that the variables whose names begin with ``prefix`` give.

    The rest of the name of such a variable, in lower case, is the name of the
    setting: with the prefix ``SHOP_``, ``SHOP_LOG_LEVEL=debug`` gives
    ``{"log_level": "debug"}``. The values are the text of the variables.
    """
    found = {}
    for variable, text in environ.items():
        if variable.startswith(prefix):
            found[variable.lstrip(prefix).lower()] = text
    return found
