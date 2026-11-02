#!/bin/zsh
# Launches the model viewer, brings its window to the front, pins the camera's look direction so every run renders
# the same frame, samples the per-stage GPU metrics, captures the lit image and optionally one intermediate target,
# and stops the viewer.
#
# usage: measure.sh <name> [viewer parameters...]
#   LOOK=lx,ly,lz     look direction to pin (default 1.0,0.0,0.0, the hall view from the spawn point)
#   TARGET="<name>"   also capture this render target, e.g. "Sun Visibility" or "Contact Shadow Trace"
#   OUT_DIR=<dir>     where the log and images go (default $TMPDIR/byte-engine-perf)
#   SANDBOX=<dir>     the sandbox workspace (default ~/development/sandbox)
#   BINARY=<path>     a saved viewer binary to run instead of the sandbox's debug build, for A/B runs without rebuilding
#   REBAKE=1          delete the viewer's baked resources first; required after any shader change
#   ROUNDS=<n>        sampling rounds of twelve forced frames each (default 3); raise it when the window stays hidden
set -u
name=$1; shift
sandbox=${SANDBOX:-$HOME/development/sandbox}
binary=${BINARY:-$sandbox/target/debug/model-viewer}
out=${OUT_DIR:-${TMPDIR:-/tmp}/byte-engine-perf}
look=${LOOK:-1.0,0.0,0.0}
mkdir -p "$out"
scripts=${0:a:h}

pkill -x "$(basename "$binary")" 2>/dev/null; sleep 1
if [[ ${REBAKE:-0} == 1 ]]; then rm -rf "$sandbox/model_viewer/resources"; fi
# The viewer resolves assets relative to its crate directory.
cd "$sandbox/model_viewer" || exit 1
( "$binary" "$@" > "$out/$name.log" 2>&1 & )
# The window opens behind the terminal and renders nothing until it is fronted; the first bake can take a while.
for i in {1..10}; do
	sleep 2
	osascript -e "tell application \"System Events\" to set frontmost of (first process whose name is \"$(basename "$binary")\") to true" >/dev/null 2>&1
done
sleep 6
# The look action's handle depends on what the viewer creates before it, so resolve it by name.
look_target=$(curl -s 'http://127.0.0.1:6680/entities?name=Lookaround' | python3 -c 'import json, sys; print(json.load(sys.stdin)[0]["target"])')
curl -s -X POST -H 'Content-Type: application/json' \
	-d "{\"type\":\"TriggerAction\",\"target\":$look_target,\"payload\":{\"type\":\"Vector3\",\"value\":[$look]}}" \
	'http://127.0.0.1:6680/messages'
sleep 1
echo "== $name"
python3 "$scripts/sample_metrics.py" 127.0.0.1:6680 "${ROUNDS:-3}"
curl -s -o "$out/${name}_lit.png" 'http://127.0.0.1:6680/screenshots?sink=0'
if [[ -n ${TARGET:-} ]]; then
	curl -s -o "$out/${name}_target.png" "http://127.0.0.1:6680/screenshots?sink=0&target=${TARGET// /%20}"
fi
echo "log errors: $(grep -ci 'error\|panic' "$out/$name.log")"
pkill -x "$(basename "$binary")"; sleep 1
