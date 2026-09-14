# Measure repeated work in changing GHI frames

Run the changing-scene benchmarks on a supported native GPU:

```sh
cargo bench -p byte-engine-ghi --bench retained_work
```

Like the rendering tests, setup selects the native backend through
`ghi::implementation` (Vulkan, Metal, or DX12). Workloads use public GHI APIs and
native shader variants. No window is required. Use `-- --list` to list cases.

Every measured frame updates CPU data, uploads an object buffer, changes object
transforms through push constants, renders every object, submits, and waits for
completion. There are no empty-frame, unchanged-frame, or unconsumed host-write
benchmarks. The question is how much metadata and temporary storage GHI rebuilds
while processing real changes.

## Compare equivalent changing workloads

Each frame renders 64 or 256 objects, with four triangle draws per object. Objects
occupy separate 16×16 tiles, so readback can check every object's latest color.
The geometry is intentionally simple: this is a controlled GHI workload, not a
representative full game scene or a GPU throughput benchmark.

All cases retain two dynamic object buffers, one descriptor set, one pipeline,
one render target, and four available command buffers. Each object buffer has
capacity for 256 colors (4096 bytes), independent of active object count. This
keeps buffer capacity constant while draw count varies.

| Case | Per-frame changes and comparison |
| --- | --- |
| `animated_objects_completed` | All active object colors and transforms change. Bind the shared object buffer once per recording. This is the primary baseline. |
| `animated_rebind_subdraws_completed` | Exactly the same changing data, transforms, and draws, with a descriptor bind before every draw. Tests retention within a changing frame. |
| `animated_four_recordings_completed` | The same objects and draws split into four sequential recordings and render passes. Later passes load existing attachment contents. Includes necessary extra pass/submission costs as well as any repeated bookkeeping. |
| `streamed_descriptors_completed` | All colors and transforms change; alternate the backing object buffer each frame and rewrite its descriptor before drawing. Every replacement is consumed and submitted. |
| `mixed_motion_completed` | All transforms change; a rotating quarter of colors changes. Retained CPU colors repopulate the active frame slot, and the complete buffer is synchronized. This is a mixed-update control, not an unchanged-frame test or proof of upload elimination. |

Changed data uses an eight-frame cycle so alternating frame slots do not
accidentally see identical data every time they are reused. No case relies on
skipping drawing, recording, or frame submission. The public API currently needs
newly uploaded resources transitioned before active Vulkan rendering; the shared
object buffer avoids introducing unsupported per-object transitions mid-pass.

## Verify before interpreting performance

In a POSIX shell, run:

```sh
GHI_BENCH_VALIDATION=1 cargo bench -p byte-engine-ghi --profile dev --bench retained_work -- --test
```

In PowerShell set `$env:GHI_BENCH_VALIDATION = "1"` first and run the Cargo command
without the environment assignment. Remove the variable before timing. Its
presence enables validation. Debug builds also assert GHI's backend error state.
Every run checks all rendered object colors after measurement, with readback
excluded from timing. Validation failures invalidate a performance baseline even
if pixels happen to match.

Device, shader, pipeline, and resource creation occur outside measurement. Each
case warms up with 32 complete changing frames before Divan measures iterations.
The harness retains its vectors and uses stack values in the measured loop.

Times are **completed-frame latency**, including CPU data preparation, upload
scheduling, recording, submission, and `Context::wait`. Two frame slots are reused,
but completion is serialized. These results are not isolated CPU encoding time,
GPU timestamp measurements, or throughput with overlapping frames. Do not remove
the wait merely to obtain a smaller number.

Divan reports Rust global-allocator activity on the benchmark thread, including
reallocations. Native driver allocations, GPU allocations, and unrelated worker
threads are not counted. Profiling adds overhead. Keep instrumentation identical
across comparisons and inspect allocation counts alongside time.

Run on an idle machine with stable power and driver settings. Record the commit,
OS/backend, GPU, driver, CPU, Rust version, command, and power settings. Repeat
comparisons in fresh processes and reverse their order. Use `--help` for filtering
and sampling controls. A filtered example is:

```sh
cargo bench -p byte-engine-ghi --bench retained_work -- animated_objects_completed
```

## Investigate retained work without assuming unchanged frames

Prioritize storage and metadata that remain reusable while their contents change:

1. Compare one versus four recordings for allocation growth. Investigate cloning
   global resource-state maps and rebuilding temporary submission arrays. Preserve
   correct ordering and transitions; the four-recording difference also includes
   real extra render-pass and submission work.
2. Compare normal animation with per-draw rebinding. Retain descriptor
   materialization when identity, version, layout, and frame slot still match,
   while continuing to upload changed object data and emit changed push constants.
3. Use descriptor replacement as an invalidation control. A retention optimization
   must still handle changed backing resources and produce the expected pixels.
4. Investigate upload batching and reusable staging storage for the all-changing
   case. The mixed-update case alone does not justify removing uploads or adding
   dirty tracking: this API synchronizes the whole buffer, and every frame needs it.

The previous empty/unchanged-frame results are superseded. They exposed code paths,
but they do not establish the priority or benefit of optimizations for this workload.

## Benchmarking guidance

- [Divan timing and setup](https://docs.rs/divan/0.1.21/divan/struct.Bencher.html): exclude fixture setup and use local timing for mutable GPU state.
- [Divan allocation profiling](https://docs.rs/divan/0.1.21/divan/struct.AllocProfiler.html): account for instrumentation and allocation visibility.
- [Criterion timing loops](https://bheisler.github.io/criterion.rs/book/user_guide/timing_loops.html): define setup, work, and destruction boundaries explicitly.
- [NVIDIA timing guidance](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.html#timing): distinguish host time from asynchronous GPU completion.
