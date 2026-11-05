# Samples the inspector's per-tick GPU metrics while forcing frames with screenshots, and prints every GPU metric
# sorted by its median. An unattended viewer (`--unattended=<hz>`, as measure.sh starts it) renders every tick on its
# own, so the screenshots only pace the rounds; an attended one whose window is hidden or behind another renders
# nothing except what they force.
#
# Rows come from `GET /events`, so no tick is missed between rounds and each round ends as soon as the frames its
# ticks submitted complete, instead of after a guessed sleep that could read rows before their GPU times arrived.
#
# usage: sample_metrics.py [host:port] [rounds] [shots_per_round]
import json
import statistics
import sys
import urllib.request

host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:6680"
rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 3
shots = int(sys.argv[3]) if len(sys.argv) > 3 else 12


def get(path):
    return urllib.request.urlopen(f"http://{host}{path}", timeout=40).read()


def events(cursor, wait):
    """Returns the cursor past the tick and GPU events after `cursor`, and those events, waiting up to `wait` ms."""
    batch = json.loads(get(f"/events?after={cursor}&wait={wait}&events=tick,gpu"))
    return batch["cursor"], batch["events"]


def round_rows(cursor):
    """Returns the cursor and the rows of the ticks that ended by now, once the frames those ticks submitted completed."""
    rows, final, last, wait = {}, set(), None, 0
    while True:
        cursor, batch = events(cursor, wait)
        for event in batch:
            if event["event"] == "closing":
                sys.exit("The viewer closed while sampling. See its log.")
            # The round ends at the last tick of its first batch; later ticks belong to the next round.
            if event["event"] in ("tick", "gpu") and (last is None or event["data"]["tick"] <= last):
                row = event["data"]
                rows[row["tick"]] = row
                # A `gpu` event, or a `tick` event that already carries GPU times, is the row's final value.
                if event["event"] == "gpu" or row["gpu"]:
                    final.add(row["tick"])
        last = max(rows, default=None) if last is None else last
        done = last is not None and all(row["frame"] is None or tick in final for tick, row in rows.items())
        if done or (wait and not batch):
            return cursor, rows
        # Frames complete as later ones are submitted, so an unattended viewer finishes them within a few ticks.
        wait = 5000


samples = {}
cursor = json.loads(get("/events"))["cursor"]
for _ in range(rounds):
    for _ in range(shots):
        get("/screenshots?sink=0")
    cursor, rows = round_rows(cursor)
    for row in rows.values():
        if row["presented"]:
            for name, value in row["gpu"].items():
                samples.setdefault(name, []).append(value)

for name in sorted(samples, key=lambda key: -statistics.median(samples[key])):
    values = samples[name]
    print(f"{name:40} n={len(values):4} p50={statistics.median(values):7.3f} min={min(values):7.3f} max={max(values):7.3f}")
