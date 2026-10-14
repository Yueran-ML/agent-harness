import io
import unittest

from config import DEFAULTS, load_config


class LoadConfigTest(unittest.TestCase):
    def test_defaults(self):
        self.assertEqual(load_config(environ={}), DEFAULTS)

    def test_file_overrides_defaults(self):
        settings = load_config(io.StringIO('{"port": 8081, "debug": true}'), environ={})
        self.assertEqual(settings["port"], 8081)
        self.assertIs(settings["debug"], True)
        self.assertEqual(settings["host"], DEFAULTS["host"])

    def test_unknown_setting_in_file(self):
        with self.assertRaises(ValueError):
            load_config(io.StringIO('{"colour": "red"}'), environ={})

    def test_environment_overrides_file(self):
        settings = load_config(
            io.StringIO('{"port": 8081, "debug": true}'),
            environ={"SHOP_PORT": "9000", "SHOP_DEBUG": "false"},
        )
        self.assertEqual(settings["port"], 9000)
        self.assertIs(settings["debug"], False)


if __name__ == "__main__":
    unittest.main()
