# A/Bs two saved model-viewer binaries: runs them interleaved, A B then B A, each unattended for a fixed number of
# ticks with the camera's look pinned, and compares the per-stage GPU medians.
#
# Each run writes every tick to NDJSON through `metrics-out` and exits through `kill-after`, so timing takes no
# screenshots and no inspector polling. The inspector is used once per run, to pin the look; ticks before the pin and
# --settle ticks after it are dropped, so loading, the first bake, and temporal histories filling up stay out of the
# medians.
#
# The GPU clock changes between runs, so each run's stages are scaled by how far its --control stage, one the change
# does not touch, sits from the control's median over all runs. Check that the control's own spread is small before
# trusting a delta. Stages whose medians are below --floor ms are marked `~0`: a timer that close to zero measures
# the timestamp, not the work.
#
# Everything goes to --out: each run's log and NDJSON, `results.json`, and `command.txt` with the exact command.
#
# usage: ab.py --a <binary> --b <binary> [--rounds 3] [--ticks 900] [--settle 120] [--rate 60]
#              [--look 1.0,0.0,0.0] [--control stage.shadow-maps] [--match stage.] [--floor 0.005]
#              [--out <dir>] [--sandbox ~/development/sandbox] [-- viewer parameters...]
#   Copy each binary out of target/ first (mbx can delete target directories), and bake the store both read before
#   the first round: a run that bakes spends its ticks loading.
import argparse
import json
import os
import shlex
import statistics
import subprocess
import sys
import time
import urllib.request

parser = argparse.ArgumentParser()
parser.add_argument("--a", required=True, help="the baseline viewer binary")
parser.add_argument("--b", required=True, help="the changed viewer binary")
parser.add_argument("--rounds", type=int, default=3, help="rounds of one A and one B run, alternating their order")
parser.add_argument("--ticks", type=int, default=900, help="ticks each run renders before it exits")
parser.add_argument("--settle", type=int, default=120, help="ticks dropped after the look is pinned")
parser.add_argument("--rate", type=int, default=60, help="the unattended rendering rate")
parser.add_argument("--look", default="1.0,0.0,0.0", help="the look direction to pin")
parser.add_argument("--control", default="stage.shadow-maps", help="a stage the change does not touch")
parser.add_argument("--match", default="", help="report only metrics whose name contains this text")
parser.add_argument("--floor", type=float, default=0.005, help="medians below this many ms are reported as ~0")
parser.add_argument("--port", type=int, default=6680, help="the inspector port the viewer listens on")
parser.add_argument("--out", default=os.path.join(os.environ.get("TMPDIR", "/tmp"), "byte-engine-ab"))
parser.add_argument("--sandbox", default=os.path.expanduser("~/development/sandbox"))
parser.add_argument("viewer", nargs="*", help="parameters passed to both binaries, after --")
arguments = parser.parse_args()

os.makedirs(arguments.out, exist_ok=True)
with open(os.path.join(arguments.out, "command.txt"), "w") as file:
    file.write(shlex.join([sys.executable, *sys.argv]) + "\n")

inspector = f"http://127.0.0.1:{arguments.port}"


def get(path):
    return urllib.request.urlopen(inspector + path, timeout=30).read()


def pin(process, name, log_path):
    """Pins the look of the viewer `process` runs and returns the first tick to keep: --settle ticks after the pin."""
    # The inspector answers before the viewer creates its look action, so wait for the action itself. Its handle
    # depends on what the viewer creates before it, so resolve it by name.
    target = None
    while target is None:
        if process.poll() is not None:
            sys.exit(f"{name} exited before its look action existed. See {log_path}.")
        try:
            entities = json.loads(get("/entities?name=Lookaround"))
            target = entities[0]["target"] if entities else None
        except OSError:
            pass
        if target is None:
            time.sleep(0.5)
    message = {"type": "TriggerAction", "target": target, "payload": {"type": "Vector3", "value": [float(v) for v in arguments.look.split(",")]}}
    request = urllib.request.Request(inspector + "/messages", json.dumps(message).encode(), {"Content-Type": "application/json"})
    urllib.request.urlopen(request, timeout=30).read()
    return json.loads(get("/metrics"))["last_tick"] + arguments.settle


def run(label, binary, round_index):
    """Runs one binary to its kill-after tick and returns its per-metric GPU medians after the pin."""
    name = f"{label}{round_index}"
    frames = os.path.join(arguments.out, f"{name}.ndjson")
    command = [
        os.path.abspath(binary),
        f"--unattended={arguments.rate}",
        f"--kill-after={arguments.ticks}",
        f"--metrics-out={frames}",
        "--render.ghi.features.counters=true",
        f"--inspector.port={arguments.port}",
        *arguments.viewer,
    ]
    with open(os.path.join(arguments.out, f"{name}.log"), "w") as log:
        # The viewer resolves assets relative to its crate directory.
        process = subprocess.Popen(command, cwd=os.path.join(arguments.sandbox, "model_viewer"), stdout=log, stderr=log)
        try:
            first = pin(process, name, log.name)
            process.wait()
        finally:
            # A failure while pinning must not leave the viewer running into the next run.
            if process.poll() is None:
                process.kill()

    samples = {}
    with open(frames) as file:
        for row in (json.loads(line) for line in file if line.strip()):
            if row["tick"] < first or not row.get("presented", True):
                continue
            for metric, value in row.get("gpu", {}).items():
                samples.setdefault(metric, []).append(value)
    if not samples:
        sys.exit(f"{name} kept no ticks after its pin. Raise --ticks above the load time plus --settle ({arguments.settle}).")
    print(f"{name}: {len(next(iter(samples.values())))} ticks", file=sys.stderr)
    return {metric: statistics.median(values) for metric, values in samples.items()}


runs = {"a": [], "b": []}
for round_index in range(arguments.rounds):
    # Alternating the order keeps a drift in clock or temperature from always favoring the same binary.
    order = [("a", arguments.a), ("b", arguments.b)]
    for label, binary in order if round_index % 2 == 0 else reversed(order):
        runs[label].append(run(label, binary, round_index))

every_run = runs["a"] + runs["b"]
if not all(arguments.control in medians for medians in every_run):
    sys.exit(f"The control {arguments.control} is missing from a run. Pass a --control stage both binaries publish.")
reference = statistics.median(medians[arguments.control] for medians in every_run)


def normalized(medians):
    scale = reference / medians[arguments.control]
    return {metric: value * scale for metric, value in medians.items()}


def spread(values):
    """The range of the run medians as a fraction of their median, which is the noise a delta has to beat."""
    middle = statistics.median(values)
    return (max(values) - min(values)) / middle if middle else 0.0


scaled = {label: [normalized(medians) for medians in label_runs] for label, label_runs in runs.items()}
metrics = sorted({metric for medians in every_run for metric in medians if arguments.match in metric})
results = {"control": arguments.control, "reference": reference, "metrics": {}}
for metric in metrics:
    a = [medians[metric] for medians in scaled["a"] if metric in medians]
    b = [medians[metric] for medians in scaled["b"] if metric in medians]
    if not a or not b:
        continue
    a_median, b_median = statistics.median(a), statistics.median(b)
    results["metrics"][metric] = {
        "a": a_median,
        "b": b_median,
        "delta": b_median - a_median,
        "relative": (b_median - a_median) / a_median if a_median else None,
        "noise": max(spread(a), spread(b)),
        "near_zero": max(a_median, b_median) < arguments.floor,
        "a_runs": a,
        "b_runs": b,
    }

with open(os.path.join(arguments.out, "results.json"), "w") as file:
    json.dump(results, file, indent=2)

print(f"control {arguments.control}: reference {reference:.3f} ms, spread {spread([m[arguments.control] for m in every_run]):.1%}")
print(f"{'metric':40} {'a':>8} {'b':>8} {'delta':>8} {'rel':>7} {'noise':>6}")
for metric, row in sorted(results["metrics"].items(), key=lambda item: -item[1]["a"]):
    if row["near_zero"]:
        print(f"{metric:40} {'~0':>8} {'~0':>8}")
        continue
    relative = f"{row['relative']:+.1%}" if row["relative"] is not None else "-"
    print(f"{metric:40} {row['a']:8.3f} {row['b']:8.3f} {row['delta']:+8.3f} {relative:>7} {row['noise']:6.1%}")
print(f"results: {os.path.join(arguments.out, 'results.json')}")
