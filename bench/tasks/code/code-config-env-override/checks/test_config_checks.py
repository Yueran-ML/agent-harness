import io
import os
import unittest
from unittest import mock

from config import DEFAULTS, load_config

FILE = (
    '{"host": "10.0.0.5", "port": 8081, "debug": true, "workers": 3,'
    ' "timeout": 2.5, "log_level": "warning"}'
)
FROM_FILE = {
    "host": "10.0.0.5",
    "port": 8081,
    "debug": True,
    "workers": 3,
    "timeout": 2.5,
    "log_level": "warning",
}


def load(environ, text=FILE):
    return load_config(io.StringIO(text), environ=environ)


class EnvironmentOverridesTest(unittest.TestCase):
    def assertSettings(self, settings, expected):
        self.assertEqual(settings, expected)
        for name, value in expected.items():
            self.assertIs(type(settings[name]), type(value), name)

    def test_every_setting_takes_the_type_of_its_default(self):
        settings = load(
            {
                "SHOP_HOST": "0.0.0.0",
                "SHOP_PORT": "9000",
                "SHOP_DEBUG": "false",
                "SHOP_WORKERS": "8",
                "SHOP_TIMEOUT": "0.75",
                "SHOP_LOG_LEVEL": "debug",
            }
        )
        self.assertSettings(
            settings,
            {
                "host": "0.0.0.0",
                "port": 9000,
                "debug": False,
                "workers": 8,
                "timeout": 0.75,
                "log_level": "debug",
            },
        )

    def test_host_alone(self):
        self.assertSettings(
            load({"SHOP_HOST": "shop.example.org"}), {**FROM_FILE, "host": "shop.example.org"}
        )

    def test_port_alone(self):
        self.assertSettings(load({"SHOP_PORT": "443"}), {**FROM_FILE, "port": 443})

    def test_without_a_file(self):
        settings = load_config(environ={"SHOP_PORT": "7000", "SHOP_WORKERS": "16"})
        self.assertSettings(settings, {**DEFAULTS, "port": 7000, "workers": 16})

    def test_file_values_stay_where_the_environment_gives_none(self):
        self.assertSettings(load({"SHOP_WORKERS": "5"}), {**FROM_FILE, "workers": 5})

    def test_true_in_any_spelling(self):
        for text in ("true", "True", "TRUE", "1", "yes", "Yes", "on", "ON"):
            with self.subTest(text=text):
                settings = load({"SHOP_DEBUG": text}, '{"debug": false}')
                self.assertIs(settings["debug"], True)

    def test_false_in_any_spelling(self):
        for text in ("false", "False", "FALSE", "0", "no", "No", "off", "OFF"):
            with self.subTest(text=text):
                settings = load({"SHOP_DEBUG": text}, '{"debug": true}')
                self.assertIs(settings["debug"], False)

    def test_float_from_a_whole_number(self):
        settings = load({"SHOP_TIMEOUT": "3"})
        self.assertEqual(settings["timeout"], 3.0)
        self.assertIs(type(settings["timeout"]), float)

    def test_invalid_values(self):
        for name, text in (
            ("SHOP_PORT", "eighty"),
            ("SHOP_WORKERS", "many"),
            ("SHOP_TIMEOUT", "soon"),
            ("SHOP_DEBUG", "maybe"),
        ):
            with self.subTest(name=name, text=text):
                with self.assertRaises(ValueError):
                    load({name: text})

    def test_variables_with_other_names_are_ignored(self):
        settings = load({"PORT": "1", "SHOPPING_PORT": "2", "MYSHOP_PORT": "3", "DEBUG": "false"})
        self.assertSettings(settings, FROM_FILE)

    def test_process_environment_by_default(self):
        with mock.patch.dict(os.environ, {"SHOP_PORT": "9123"}):
            settings = load_config(io.StringIO(FILE))
        self.assertEqual(settings["port"], 9123)


if __name__ == "__main__":
    unittest.main()
