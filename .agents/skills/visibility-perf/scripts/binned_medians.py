# Prints each run's per-stage GPU medians, split by the GPU's clock state.
#
# A forced-frame run often changes clock state partway, and its overall median then lands between two states and
# matches neither. This groups each run's frames by a regime key, a stage the change does not touch: it takes the
# densest window of key values no wider than --gap as one state, removes those frames, and repeats, so frames caught
# between two states end up in small groups that are dropped. With --at, each run instead gets one group per given key
# value, the frames within half of --gap of it, so the columns of an A/B line up. Compare columns whose key medians
# agree within 1%, and keep an unchanged stage in view as a control: the key varies almost continuously on the M4, and
# the population a temporal pass works on drifts between runs, so a stage the change does not touch is what shows
# whether two columns are comparable.
#
# usage: binned_medians.py [--key stage.shadow-maps] [--gap 0.06] [--min 20] [--at 0.77,1.04] [--match text]
#                          [name=]frames.ndjson...
#   Each file holds the NDJSON of GET /metrics/frames?limit=1024; a file without a name is labelled by its directory.
import argparse
import bisect
import json
import os
import statistics

parser = argparse.ArgumentParser()
parser.add_argument("--key", default="stage.shadow-maps", help="the regime key, a stage the change does not touch")
parser.add_argument("--gap", type=float, default=0.06, help="widest relative spread of the key within one state")
parser.add_argument("--min", type=int, default=20, help="fewest frames a state needs to be shown")
parser.add_argument("--match", default="", help="show only metrics whose name contains this text")
parser.add_argument("--at", default="", help="comma-separated key values to group each run's frames around")
parser.add_argument("runs", nargs="+")
arguments = parser.parse_args()


def states(frames):
    """Splits frames into groups of one clock state each, densest first."""
    if arguments.at:
        centers = [float(center) for center in arguments.at.split(",")]
        groups = [
            [gpu for gpu in frames if abs(gpu[arguments.key] / center - 1) <= arguments.gap / 2] for center in centers
        ]
        return [group for group in groups if len(group) >= arguments.min]
    frames = sorted(frames, key=lambda gpu: gpu[arguments.key])
    groups = []
    while len(frames) >= arguments.min:
        keys = [gpu[arguments.key] for gpu in frames]
        start, end = max(
            ((i, bisect.bisect_right(keys, keys[i] * (1 + arguments.gap))) for i in range(len(keys))),
            key=lambda window: window[1] - window[0],
        )
        if end - start < arguments.min:
            break
        groups.append(frames[start:end])
        frames = frames[:start] + frames[end:]
    return groups


columns = []
for run in arguments.runs:
    label, _, path = run.rpartition("=")
    label = label or os.path.basename(os.path.dirname(os.path.abspath(path)))
    frames = []
    for line in open(path):
        try:
            gpu = json.loads(line).get("gpu") or {}
        except ValueError:
            continue
        if arguments.key in gpu:
            frames.append(gpu)
    for group in sorted(states(frames), key=lambda group: statistics.median(gpu[arguments.key] for gpu in group)):
        values = {}
        for gpu in group:
            for name, value in gpu.items():
                values.setdefault(name, []).append(value)
        medians = {name: statistics.median(found) for name, found in values.items() if len(found) >= 3}
        columns.append((f"{label[-9:]}@{medians[arguments.key]:.3f}", len(group), medians))

if not columns:
    raise SystemExit(f"No run has {arguments.min} frames in one state of {arguments.key}.")
names = {name for _, _, medians in columns for name in medians if arguments.match in name}
names.discard(arguments.key)
print(f"{'':40}" + "".join(f"{title:>17}" for title, _, _ in columns))
print(f"{'frames':40}" + "".join(f"{count:17d}" for _, count, _ in columns))
for name in [arguments.key] + sorted(names, key=lambda name: -columns[0][2].get(name, 0.0)):
    print(f"{name:40}" + "".join(f"{medians.get(name, float('nan')):17.3f}" for _, _, medians in columns))
