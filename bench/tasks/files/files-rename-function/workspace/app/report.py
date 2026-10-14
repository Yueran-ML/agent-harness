from app import data


def summary():
    # fetch_data returns a list of dicts
    records = data.fetch_data()
    return {"count": len(records), "source": "fetch_data"}
