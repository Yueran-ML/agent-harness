import json
import tempfile
import unittest
from pathlib import Path

from app.data import load_records


class FetchDataTest(unittest.TestCase):
    def test_fetch_data_reads_every_record(self):
        with tempfile.TemporaryDirectory() as tmp:
            store = Path(tmp) / "records.json"
            store.write_text(json.dumps([{"id": 1, "name": "a"}]))
            self.assertEqual(load_records(store), [{"id": 1, "name": "a"}])
