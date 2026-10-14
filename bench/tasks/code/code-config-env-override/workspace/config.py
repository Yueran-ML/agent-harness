"""The settings of the shop service.

Every setting has a default. A JSON file can override the defaults, and an
environment variable named ``SHOP_`` followed by the name of a setting in
capitals, such as ``SHOP_LOG_LEVEL`` for ``log_level``, overrides both.
"""

import json
import os

from envvars import read_overrides

DEFAULTS = {
    "host": "127.0.0.1",
    "port": 8080,
    "debug": False,
    "workers": 2,
    "timeout": 5.0,
    "log_level": "info",
}
"""Every setting, with the value it has when neither the file nor the environment gives one."""

PREFIX = "SHOP_"
"""How the name of every environment variable that overrides a setting begins."""


def load_config(stream=None, environ=None):
    """Return the settings.

    Args:
        stream: An open JSON file that gives some of the settings, or ``None``.
        environ: The environment variables, ``os.environ`` if not given.

    Raises:
        ValueError: The file gives a setting that there is none of.
    """
    settings = dict(DEFAULTS)
    if stream is not None:
        from_file = json.load(stream)
        unknown = sorted(set(from_file) - set(DEFAULTS))
        if unknown:
            raise ValueError(f"unknown settings: {', '.join(unknown)}")
        settings.update(from_file)
    overrides = read_overrides(os.environ if environ is None else environ, PREFIX)
    for name, text in overrides.items():
        if name in settings:
            settings[name] = text
    return settings


def load_config_file(path, environ=None):
    """Return the settings, with those that the JSON file at ``path`` gives."""
    with open(path, encoding="utf-8") as stream:
        return load_config(stream, environ)
