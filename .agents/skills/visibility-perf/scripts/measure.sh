#!/bin/zsh
# Launches the model viewer unattended, so its window renders every tick in the background without being fronted and
# without disturbing the desktop, pins the camera's look direction so every run renders the same frame, samples the per-stage GPU metrics, captures the lit image and optionally one intermediate target,
# and stops the viewer.
#
# usage: measure.sh <name> [viewer parameters...]
#   LOOK=lx,ly,lz     look direction to pin (default 1.0,0.0,0.0, the hall view from the spawn point)
#   TARGET="<name>"   also capture this render target, e.g. "Sun Visibility" or "Contact Shadow Trace"
#   OUT_DIR=<dir>     where the log and images go (default $TMPDIR/byte-engine-perf)
#   SANDBOX=<dir>     the sandbox workspace (default ~/development/sandbox)
#   BINARY=<path>     a saved viewer binary to run instead of the sandbox's debug build, for A/B runs without rebuilding
#   REBAKE=1          delete the viewer's baked resources first; required after any shader change
#   RATE=<hz>         the unattended rendering rate (default 60); raise it above what the GPU reaches to render back to back
#   ROUNDS=<n>        sampling rounds of twelve captures each (default 3)
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
# Release builds leave GPU counters off, and without them no stage time is sampled.
( "$binary" --unattended="${RATE:-60}" --render.ghi.features.counters=true "$@" > "$out/$name.log" 2>&1 & )
# The inspector answers once the viewer runs; the first bake can take a while after that.
until curl -sf 'http://127.0.0.1:6680/configuration' > /dev/null; do sleep 1; done
cursor=$(curl -s 'http://127.0.0.1:6680/events' | jq -r .cursor)
# Returns once `$1` more ticks ended, or fails when the viewer closes; it paces on the viewer instead of a guessed sleep.
wait_ticks() {
	local left=$1 batch
	while (( left > 0 )); do
		batch=$(curl -s "http://127.0.0.1:6680/events?after=$cursor&wait=5000&events=tick") || return 1
		cursor=$(jq -r .cursor <<<"$batch")
		jq -e 'any(.events[]; .event == "closing")' <<<"$batch" > /dev/null && return 1
		(( left -= $(jq '[.events[] | select(.event == "tick")] | length' <<<"$batch") ))
	done
}
# The viewer creates its look action after loading, so wait for the action itself, one tick at a time. Its handle
# depends on what the viewer creates before it, so resolve it by name.
while [[ $(curl -s 'http://127.0.0.1:6680/entities?name=Lookaround') == '[]' ]]; do wait_ticks 1 || exit 1; done
look_target=$(curl -s 'http://127.0.0.1:6680/entities?name=Lookaround' | jq '.[0].target')
curl -s -X POST -H 'Content-Type: application/json' \
	-d "{\"type\":\"TriggerAction\",\"target\":$look_target,\"payload\":{\"type\":\"Vector3\",\"value\":[$look]}}" \
	'http://127.0.0.1:6680/messages'
# The pin applies on the next tick and shows in the frame after it.
wait_ticks 2 || exit 1
echo "== $name"
python3 "$scripts/sample_metrics.py" 127.0.0.1:6680 "${ROUNDS:-3}"
curl -s -o "$out/${name}_lit.png" 'http://127.0.0.1:6680/screenshots?sink=0'
if [[ -n ${TARGET:-} ]]; then
	curl -s -o "$out/${name}_target.png" "http://127.0.0.1:6680/screenshots?sink=0&target=${TARGET// /%20}"
fi
echo "log errors: $(grep -ci 'error\|panic' "$out/$name.log")"
pkill -x "$(basename "$binary")"; sleep 1
