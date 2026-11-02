# Samples the inspector's per-tick GPU metrics while forcing frames with screenshots, and prints every GPU metric
# sorted by its median. An unattended viewer (`--unattended=<hz>`, as measure.sh starts it) renders every tick on its
# own, so the screenshots only pace the rounds; an attended one whose window is hidden or behind another renders
# nothing except what they force. The inspector keeps only 1024 ticks, so samples are accumulated here.
#
# usage: sample_metrics.py [host:port] [rounds] [shots_per_round]
import json
import statistics
import sys
import time
import urllib.request

host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:6680"
rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 3
shots = int(sys.argv[3]) if len(sys.argv) > 3 else 12


def get(path):
    return urllib.request.urlopen(f"http://{host}{path}", timeout=30).read()


samples = {}
for _ in range(rounds):
    since = json.loads(get("/metrics"))["last_tick"]
    for _ in range(shots):
        get("/screenshots?sink=0")
    time.sleep(0.3)
    rows = get(f"/metrics/frames?since={since}&presented=true").decode().splitlines()
    for row in (json.loads(line) for line in rows if line.strip()):
        for name, value in row.get("gpu", {}).items():
            samples.setdefault(name, []).append(value)

for name in sorted(samples, key=lambda key: -statistics.median(samples[key])):
    values = samples[name]
    print(f"{name:40} n={len(values):4} p50={statistics.median(values):7.3f} min={min(values):7.3f} max={max(values):7.3f}")
