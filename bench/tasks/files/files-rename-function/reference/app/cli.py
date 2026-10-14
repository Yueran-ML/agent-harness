import argparse

from app.data import load_records, fetch_data_cached


def main(argv=None):
    parser = argparse.ArgumentParser(description="Show the records in the store.")
    parser.add_argument("--cached", action="store_true")
    args = parser.parse_args(argv)
    loader = fetch_data_cached if args.cached else load_records
    for record in loader():
        print(record["id"], record["name"])
