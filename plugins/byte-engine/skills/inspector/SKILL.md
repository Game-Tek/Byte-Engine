---
name: inspector
description: "Drive and observe a running Byte Engine application through its HTTP inspector: find entities, post messages such as TriggerAction to play-test input, wait for ticks, GPU times, or settings to take effect without sleeping, capture screenshots of the window or of any render target, show debug views, read CPU and GPU metrics, read debug data streams, and close the app. Start it with `--unattended=<frames per second>` so it renders on its own in the background without disturbing the desktop. Use when the user wants to check what a Byte Engine app does at runtime, test gameplay or input without a human at the keyboard, see a frame or an intermediate render target, or measure frame time."
---

# Drive a running Byte Engine application

Every headed Byte Engine application serves an HTTP inspector on loopback while it runs. It has no
authentication and listens only on `127.0.0.1` and `::1`. Use it instead of guessing from source code what the
application does.

## Start the application so you can drive it

1. Pick a port per instance and pass it on the command line. The default is 6680. If the default port is in use,
   the inspector quietly moves to a free port and logs `HTTP inspector port 6680 is in use, so the inspector
   serves on port N instead.` A port you set explicitly that is in use stops startup, which is easier to notice.
2. Run the application in the background with `--unattended=<frames per second>`, and send its log to a file so
   you can read errors and the port later:

   ```bash
   cargo run -- --unattended=30 --inspector.port=6699 > app.log 2>&1 &
   ```

   With `--unattended`, the window never shows or takes focus, so the user keeps working undisturbed. Every tick
   still renders at the rate you give, even while the window can't be seen or the screen is locked, so you never
   force frames or bring the window to the front. Each tick advances time by exactly one frame (1/30 s above), so
   tick N shows the same simulated time in every run. The rate is required: 30 is enough to watch behavior, 60
   matches what a player sees. Leave the flag out only when the user wants to watch or play the window themselves.

   Add `--metrics-out=metrics.json` (summary) or `--metrics-out=metrics.ndjson` (every retained tick) to keep
   metrics after the application closes, and `--debug.streams=streams.ndjson` to collect debug data streams.
   Any startup parameter can be passed as `--name=value`.

   To measure GPU time, also pass `--render.ghi.features.counters=true`. Release builds leave GPU counters off, and
   without them `/metrics` and `gpu` events carry CPU spans only. Check that the first sample has GPU times before
   timing anything.
3. Wait until the inspector answers before you send anything. This is the only wait that needs a retry loop: once
   the inspector answers, wait with `GET /events` instead (see [Wait for something to happen](#wait-for-something-to-happen)):

   ```bash
   until curl -sf http://127.0.0.1:6699/configuration > /dev/null; do sleep 1; done
   ```

4. When you are done, close it through the inspector so `metrics-out` gets written. The response confirms the
   request, not that the process has exited:

   ```bash
   curl -X DELETE http://127.0.0.1:6699/
   ```

## Choose the route for the question

| You want to | Call |
| --- | --- |
| Find an entity's handle, types, name, or position | `GET /entities?name=<exact name>` or `GET /entities?type=<full Rust type path>` |
| Learn which messages the app accepts and their payload shapes | `GET /messages/types` |
| Move, delete, or trigger an input action on an entity | `POST /messages` |
| See the final frame | `GET /screenshots?sink=0` |
| See a render target, or a target right after one pass | `GET /screenshots?sink=0&target=<name>[&pass=<pass>]` |
| Capture several images from the same frame | `POST /screenshots` |
| List debug views and the slot each shown view holds | `GET /debug-views` |
| Check whether a runtime setting took effect, and why not | `GET /configuration` |
| Get frame-time percentiles per CPU span and GPU pass | `GET /metrics?presented=true` |
| Get per-tick rows | `GET /metrics/frames?since=<tick>&limit=<n>` |
| Wait for ticks, GPU times, a setting to take effect, or the app to close | `GET /events?after=<cursor>&wait=<ms>` |

Filter on the server instead of downloading everything: use `?name=` and `?type=` on `/entities`, and `since`
and `limit` on `/metrics/frames`. Pipe JSON through `jq` to keep only the fields you need.

## Wait for something to happen

Don't `sleep` and then poll. `GET /events` returns the moment a tick ends, a frame's GPU times arrive, a setting
changes state, or the application starts closing. You learn the exact tick something happened on, you don't wait
longer than needed, and you can't read a stale row.

1. Get a cursor. The response also lists the state of every configuration update so far:

   ```bash
   cursor=$(curl -s http://127.0.0.1:6699/events | jq -r .cursor)
   ```

2. Pass the latest cursor as `after` and how long to wait in milliseconds, up to 30000. Keep only the events you
   need with `events=tick`, `gpu`, or `configuration`, comma-separated. Always replace the cursor with the
   response's `cursor`, even when `events` is empty:

   ```bash
   # Waits until `n` more ticks end, and stops early if the application closes.
   wait_ticks() {
     local left=$1 batch
     while (( left > 0 )); do
       batch=$(curl -s "http://127.0.0.1:6699/events?after=$cursor&wait=5000&events=tick") || return 1
       cursor=$(jq -r .cursor <<<"$batch")
       jq -e 'any(.events[]; .event == "closing")' <<<"$batch" > /dev/null && return 1
       left=$(( left - $(jq '[.events[] | select(.event == "tick")] | length' <<<"$batch") ))
     done
   }
   ```

| To wait for | Watch for |
| --- | --- |
| The next N frames, such as after you post a message | N `tick` events; each carries the tick's metric row |
| A frame's GPU times | A `gpu` event for its tick, or a `tick` event whose `gpu` map isn't empty |
| A setting to take effect | A `configuration` event for its `parameter` whose `state.status` isn't `pending` |
| The application to close | A `closing` event, after which every request returns at once |

- A `lagged` event means more than 1,024 ticks ended since your last request. Read what you missed from
  `/metrics/frames?since=<tick>`.
- At most four requests can wait at once, and a fifth gets `503`. Keep one wait loop per tool.
- Waiting doesn't wake the application. An attended app that renders on demand and has nothing to draw ends no
  ticks, so the wait ends empty. Unattended runs end a tick at every period.

## Play-test input

1. Find the action's handle by its name. Action names are chosen by the application, such as `Move` or `Jump`.
   The target is the action entity, not the character it moves:

   ```bash
   curl -s 'http://127.0.0.1:6699/entities?name=Jump' | jq '.[].target'
   ```

2. Check the payload shape in `GET /messages/types`, then post the value. The value's kind must match the
   action's declared type, such as `Boolean`, `Float`, `Integer`, or a vector:

   ```bash
   curl -s -X POST http://127.0.0.1:6699/messages \
     -H 'content-type: application/json' \
     -d '{"type":"TriggerAction","target":42,"payload":{"type":"Boolean","value":true}}'
   ```

3. Check the effect. A `204` means the message was published, not that anything reacted to it. Wait for the
   ticks the reaction needs with `wait_ticks` from [Wait for something to happen](#wait-for-something-to-happen),
   instead of sleeping. Then read the affected entity's `position` from `/entities`, take a screenshot, or read its
   debug data stream.

`TransformationUpdate` takes a complete transform: `position` and `scale` as three numbers and `orientation` as a
quaternion of four. `Delete` and `Destroy` take a `null` payload.

`GET /messages` drains a cursor that every client shares, so two tools that both poll it each see only part of
the stream. It reports message ranges by Rust type, not payloads.

## Capture images

- Save the response to a file and open the file to look at it. The response headers give the frame number, sink,
  texture format, and size (`X-Byte-Engine-Frame`, `X-Byte-Engine-Width`, and so on). The inspector sends header
  names in lowercase (`x-byte-engine-frame`), so match them without regard to case.
- `format=png` is an 8-bit preview that clamps HDR values. Use `format=exr` to read HDR values or compare images
  numerically, and `format=raw` for the bytes the GPU stores.
- A sink is a window, numbered from 0. An application without a window has no sink, and the request returns 404.
- `previous=true` with a history `target` reads what the previous frame wrote. It holds no usable data on a sink's
  first frame or right after a resize.
- One request takes 1 to 16 captures, waits up to 5 seconds for a frame, and at most 8 requests can wait at once.
  A request wakes the application loop, so it renders a frame even when nothing else would.

```bash
curl -s -X POST http://127.0.0.1:6699/screenshots -H 'content-type: application/json' \
  -d '{"captures":[{"sink":0},{"sink":0,"target":"Diffuse Radiance History","format":"exr"}]}' \
  -o captures.multipart
```

The batch response is `multipart/form-data`, one part per capture in request order.

## Show a debug view

Debug views show values a pass computes, such as normals or motion vectors. They exist in debug builds, and in a
release build only when `config.json` sets `render.debug-views.enabled` and the project was baked again.

1. `GET /debug-views` lists the views as `<pass>.<view>`, and says whether the build has them and how many can
   be shown at once.
2. Start the application with `--render.debug.view.<pass>.<view>=true`. Add `--render.debug.targets=<n>` to
   show more than four at once.
3. `GET /configuration` reports whether the request was applied, and the reason when it was refused. To know the
   moment it settles, wait for its `configuration` event with `GET /events?events=configuration`.
4. Capture one view at full resolution by the slot `/debug-views` lists:
   `GET /screenshots?sink=0&target=Debug+View+<slot>`.

## Measure frame time

- Run with `--unattended=<rate>` so every tick renders, whether or not the window can be seen. Still pass
  `presented=true` to `/metrics` and `/metrics/frames`: in an attended run, ticks of a hidden or covered window
  present nothing and skew the numbers.
- An unattended run sleeps to hold its rate, so wall-clock frame time is the period. Read the CPU spans and GPU
  passes for cost, not `dt`. To measure the fastest a frame can go, pass a rate above what the machine reaches,
  such as `--unattended=1000`.
- GPU metrics are `frame`, `scene.<pipeline>`, `pass.<name>`, and `stage.<name>`, with `@<sink>` on sinks after
  the first. They exist only while `render.ghi.features.counters` is on, which is the default in debug builds only.
  CPU metrics are named after `tracing` spans, such as `GraphicsApplication::tick`. Every time is in
  milliseconds.
- Only the latest 1,024 ticks are kept. To follow a long run, loop on `GET /events?events=tick,gpu` instead of
  polling `/metrics/frames`. Each `tick` event is a row as soon as the tick ends, and no rows are lost between
  requests unless a `lagged` event says so.
- GPU times arrive when the frame completes, a few ticks after the tick that submitted it, so the newest rows don't
  have them yet. A `gpu` event repeats the row once they do; replace the row you stored for that tick.

## Follow one entity with debug data streams

A debug data stream reports what a system knows about one entity, one JSON line per update. The application opens
it by creating a `DebugDataStream` entity for the entity it wants to follow. You choose where the records go with
`--debug.streams=<file, stdout, stderr, or log>`. Add `--debug.streams.changes-only=true` to write a line only
when the system's state changed. Each record's `tick` matches the `tick` in `/metrics/frames`, and its `entity`
matches `target` in `/entities`.

```bash
jq -c 'select(.name == "Character") | .data' streams.ndjson | tail -20
```

## When something doesn't work

| Symptom | Likely cause |
| --- | --- |
| Nothing answers on the port | The application is still starting, it crashed (read its log), or the inspector moved to another port. |
| `presented_ticks` is 0, or screenshots time out | The run is attended and the window is hidden, minimized, or covered, or the screen is locked. Start it again with `--unattended=<rate>`. |
| The first message of a run takes about a second | On macOS, a covered window in an attended run can block the first frame it presents. Start it with `--unattended=<rate>`, or send a warm-up message and wait for its `tick` event before a timed sequence. |
| A wait on `/events` returns `503` | Four other requests already wait. Close the extra wait loops, or retry without `wait`. |
| Startup stops with "The `unattended` parameter has no valid rendering rate" | `--unattended` needs a positive rate, such as `--unattended=30`. |
| A setting has no effect | `GET /configuration` lists it as pending or not set, with the reason. Some settings keep their startup value. |
| The `metrics-out` file holds one JSON object instead of a row per tick | Only a path ending in `.ndjson` gets every retained tick; any other extension, `.jsonl` included, gets the summary. |
| A run reports a much shorter frame than expected | Something it should draw didn't load. Read the log for load warnings and capture a screenshot before trusting the number. |
| An entity query returns `[]` | Names are exact and case-sensitive, and `type` needs the full Rust path, such as `byte_engine::rendering::window::Window`. |

## Reference

Read these pages for the exact rules. Each is served as Markdown:

- Inspector, metrics, messages, events, and screenshots: https://byte-engine.0x44491229.dev/docs/reference/inspector.md
- Waiting for inspector events from a script: https://byte-engine.0x44491229.dev/docs/use/inspector-events.md
- Every startup parameter: https://byte-engine.0x44491229.dev/docs/reference/parameters.md
- Unattended runs, pacing, and tick time: https://byte-engine.0x44491229.dev/docs/reference/time.md
- Debug views: https://byte-engine.0x44491229.dev/docs/use/debug-views.md
- Debug data streams: https://byte-engine.0x44491229.dev/docs/use/debug-data-streams.md and https://byte-engine.0x44491229.dev/docs/reference/debug-data-streams.md
- Index of every page: https://byte-engine.0x44491229.dev/llms.txt
