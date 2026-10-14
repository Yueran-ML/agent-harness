from app import data


def summary():
    # load_records returns a list of dicts
    records = data.load_records()
    return {"count": len(records), "source": "load_records"}
