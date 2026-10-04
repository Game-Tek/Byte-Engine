//! Per-tick performance samples for tooling that reads them over the inspector.
//!
//! [`Metrics`] keeps one [`TickSample`] per application tick in a bounded ring: the tick's clock, whether it reached
//! the screen, the CPU time spent in every `tracing` span the engine opens, and the GPU time of every counter the
//! renderer records. Spans reach it through [`MetricsLayer`], which the application installs as its global `tracing`
//! subscriber, so the spans Tracy shows and the numbers the inspector serves come from the same call sites. GPU
//! times arrive late, once the frame that recorded them completes, and land in the tick that submitted that frame.
//!
//! Read the ring as summaries with [`Metrics::summary`] or row by row with [`Metrics::ticks_since`]. The HTTP
//! inspector serves both as `GET /metrics` and `GET /metrics/frames`.

use std::{
	cell::RefCell,
	collections::HashMap,
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

use serde::{Serialize, Serializer, ser::SerializeMap};
use smallvec::SmallVec;
use tracing::{
	Metadata,
	callsite::Identifier,
	span::{Attributes, Id},
	subscriber::Interest,
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};
use utils::sync::{Mutex, RwLock};

/// The number of distinct metrics one application can track, across CPU spans and GPU counters.
pub const METRIC_CAPACITY: usize = 128;

/// The number of ticks the ring retains before the oldest one is overwritten.
pub const TICK_CAPACITY: usize = 1024;

/// A value a tick never measured.
const ABSENT: u64 = u64::MAX;

/// The `MetricId` struct names one registered metric so the hot paths index arrays instead of comparing names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MetricId(u16);

/// The `MetricKind` enum separates the clock a metric came from, since CPU and GPU times of one tick do not add up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
	/// Time the CPU spent inside spans of one name during the tick, summed over every entry on every thread.
	Cpu,
	/// Time the GPU spent between the start and end of one counter in the frame the tick submitted.
	Gpu,
}

/// The `TickSample` struct holds everything one tick measured, with each metric at its registered index.
#[derive(Clone)]
pub struct TickSample {
	/// The zero-based application tick this row belongs to.
	pub tick: u64,
	/// The renderer frame the tick submitted, which screenshots also report, or `None` when it rendered nothing.
	pub frame: Option<u64>,
	/// Wall-clock time since the application started, measured when the tick ended.
	pub time: Duration,
	/// The frame delta the tick advanced the application clock by.
	pub delta: Duration,
	/// Whether a window acquired a swapchain image this tick, so the frame can reach the screen.
	pub presented: bool,
	values: [u64; METRIC_CAPACITY],
}

impl TickSample {
	/// Returns the value `metric` measured this tick, when it did.
	pub fn value(&self, metric: MetricId) -> Option<Duration> {
		let value = self.values[metric.0 as usize];
		(value != ABSENT).then(|| Duration::from_nanos(value))
	}
}

/// The `TickClock` struct carries the per-tick facts only the application loop knows when it ends a tick.
#[derive(Clone, Copy, Debug)]
pub struct TickClock {
	/// The zero-based tick that is ending.
	pub tick: u64,
	/// The renderer frame the tick submitted, if any.
	pub frame: Option<u64>,
	/// Wall-clock time since the application started.
	pub time: Duration,
	/// The frame delta of the tick.
	pub delta: Duration,
	/// Whether a window acquired a swapchain image this tick.
	pub presented: bool,
}

/// A registered metric's name and clock.
struct Registration {
	name: Box<str>,
	kind: MetricKind,
}

/// The state the main thread owns: registrations and the ring of finished ticks.
struct Inner {
	registrations: Vec<Registration>,
	ring: Vec<TickSample>,
	/// The ring index the next finished tick is written to.
	next: usize,
	/// The time of the first tick that could reach the screen.
	time_to_first_frame: Option<Duration>,
}

impl Inner {
	/// Returns the retained ticks oldest first: once the ring is full, `next` is also the oldest row.
	fn ticks(&self) -> impl Iterator<Item = &TickSample> {
		self.ring[self.next..].iter().chain(self.ring[..self.next].iter())
	}
}

/// The `Metrics` struct collects per-tick CPU and GPU timings and serves them to inspection transports.
///
/// Create one per application, hand a clone of the [`Arc`] to the [`MetricsLayer`] and the renderer, and call
/// [`Self::end_tick`] once per tick after the tick's own span closed. Read it back through [`Self::summary`] and
/// [`Self::ticks_since`].
pub struct Metrics {
	/// Nanoseconds every thread accumulated into each CPU metric since the last tick ended.
	cpu_accumulators: [AtomicU64; METRIC_CAPACITY],
	inner: Mutex<Inner>,
}

impl Default for Metrics {
	fn default() -> Self {
		Self::new()
	}
}

impl Metrics {
	/// Creates an empty collector that holds [`TICK_CAPACITY`] ticks of [`METRIC_CAPACITY`] metrics.
	pub fn new() -> Self {
		Self {
			cpu_accumulators: std::array::from_fn(|_| AtomicU64::new(0)),
			inner: Mutex::new(Inner {
				registrations: Vec::with_capacity(METRIC_CAPACITY),
				ring: Vec::with_capacity(TICK_CAPACITY),
				next: 0,
				time_to_first_frame: None,
			}),
		}
	}

	/// Registers a metric by name, or returns the id it already has, so spans with one name share one metric.
	///
	/// Returns `None` once [`METRIC_CAPACITY`] distinct metrics exist, or when the same name was registered with the
	/// other clock, which keeps CPU and GPU columns from mixing.
	pub fn register(&self, kind: MetricKind, name: &str) -> Option<MetricId> {
		let mut inner = self.inner.lock();
		if let Some(index) = inner
			.registrations
			.iter()
			.position(|registration| &*registration.name == name)
		{
			return (inner.registrations[index].kind == kind).then_some(MetricId(index as u16));
		}
		if inner.registrations.len() >= METRIC_CAPACITY {
			log::warn!(
				"Metric {name} is not tracked. The most likely cause is that the application registered more than {METRIC_CAPACITY} metrics."
			);
			return None;
		}
		inner.registrations.push(Registration { name: name.into(), kind });
		Some(MetricId((inner.registrations.len() - 1) as u16))
	}

	/// Adds CPU time to `metric` for the tick in progress. Any thread may call it.
	pub fn add_cpu(&self, metric: MetricId, duration: Duration) {
		self.cpu_accumulators[metric.0 as usize].fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
	}

	/// Stores the GPU time `metric` measured in renderer frame `frame`, which an earlier tick submitted.
	///
	/// The sample is dropped when that tick already left the ring.
	pub fn set_gpu(&self, frame: u64, metric: MetricId, duration: Duration) {
		let mut inner = self.inner.lock();
		if let Some(sample) = inner.ring.iter_mut().rev().find(|sample| sample.frame == Some(frame)) {
			sample.values[metric.0 as usize] = duration.as_nanos() as u64;
		}
	}

	/// Closes the tick: moves the accumulated CPU time into a new row and clears the accumulators.
	///
	/// Call it after the tick's outermost span closed, otherwise that span's time lands in the next tick.
	pub fn end_tick(&self, clock: TickClock) {
		let mut sample = TickSample {
			tick: clock.tick,
			frame: clock.frame,
			time: clock.time,
			delta: clock.delta,
			presented: clock.presented,
			values: [ABSENT; METRIC_CAPACITY],
		};
		let mut inner = self.inner.lock();
		for (index, registration) in inner.registrations.iter().enumerate() {
			if registration.kind == MetricKind::Cpu {
				sample.values[index] = self.cpu_accumulators[index].swap(0, Ordering::Relaxed);
			}
		}
		if clock.presented && inner.time_to_first_frame.is_none() {
			inner.time_to_first_frame = Some(clock.time);
		}
		if inner.ring.len() < TICK_CAPACITY {
			inner.ring.push(sample);
		} else {
			let next = inner.next;
			inner.ring[next] = sample;
		}
		inner.next = (inner.next + 1) % TICK_CAPACITY;
	}

	/// Returns the retained ticks after `since`, oldest first, or every retained tick when `since` is `None`.
	pub fn ticks_since(&self, since: Option<u64>) -> Vec<Sample> {
		let inner = self.inner.lock();
		let names = Arc::new(Names::from_registrations(&inner.registrations));
		inner
			.ticks()
			.filter(|sample| since.is_none_or(|since| sample.tick > since))
			.map(|sample| Sample {
				names: Arc::clone(&names),
				sample: sample.clone(),
			})
			.collect()
	}

	/// Summarizes every metric over the retained ticks after `since`, optionally over presented ticks only.
	pub fn summary(&self, since: Option<u64>, presented_only: bool) -> Summary {
		let inner = self.inner.lock();
		let rows = inner
			.ticks()
			.filter(|sample| since.is_none_or(|since| sample.tick > since))
			.filter(|sample| !presented_only || sample.presented)
			.collect::<Vec<_>>();
		let mut values = Vec::with_capacity(rows.len());
		let mut collect = |pick: &dyn Fn(&TickSample) -> u64| -> Option<Statistics> {
			values.clear();
			values.extend(rows.iter().map(|sample| pick(sample)).filter(|value| *value != ABSENT));
			Statistics::of(&mut values)
		};
		let delta = collect(&|sample| sample.delta.as_nanos() as u64);
		let metrics = inner
			.registrations
			.iter()
			.enumerate()
			.filter_map(|(index, registration)| {
				let statistics = collect(&|sample| sample.values[index])?;
				Some(MetricSummary {
					name: registration.name.clone(),
					kind: registration.kind,
					statistics,
				})
			})
			.collect();
		Summary {
			ticks: rows.len() as u64,
			first_tick: rows.first().map(|sample| sample.tick),
			last_tick: rows.last().map(|sample| sample.tick),
			presented_ticks: rows.iter().filter(|sample| sample.presented).count() as u64,
			time_to_first_frame: inner.time_to_first_frame,
			delta,
			metrics,
		}
	}
}

/// Metric names in id order, shared by the rows of one read so each row serializes without copying them.
struct Names {
	names: Vec<(Box<str>, MetricKind)>,
}

impl Names {
	fn from_registrations(registrations: &[Registration]) -> Self {
		Self {
			names: registrations
				.iter()
				.map(|registration| (registration.name.clone(), registration.kind))
				.collect(),
		}
	}
}

/// The `Sample` struct is one retained tick with the names its values serialize under.
///
/// It serializes as one JSON object: `tick`, `frame`, `time`, `dt`, `presented`, and a `cpu` and a `gpu` object of
/// metric name to milliseconds, listing only what the tick measured.
pub struct Sample {
	names: Arc<Names>,
	sample: TickSample,
}

impl Sample {
	/// Returns the measured tick.
	pub fn tick(&self) -> &TickSample {
		&self.sample
	}
}

impl Serialize for Sample {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut row = serializer.serialize_map(Some(7))?;
		row.serialize_entry("tick", &self.sample.tick)?;
		row.serialize_entry("frame", &self.sample.frame)?;
		row.serialize_entry("time", &milliseconds(self.sample.time))?;
		row.serialize_entry("dt", &milliseconds(self.sample.delta))?;
		row.serialize_entry("presented", &self.sample.presented)?;
		for kind in [MetricKind::Cpu, MetricKind::Gpu] {
			let key = match kind {
				MetricKind::Cpu => "cpu",
				MetricKind::Gpu => "gpu",
			};
			row.serialize_entry(
				key,
				&Values {
					names: &self.names,
					values: &self.sample.values,
					kind,
				},
			)?;
		}
		row.end()
	}
}

/// The measured values of one clock in one tick, as a name-to-milliseconds map.
struct Values<'a> {
	names: &'a Names,
	values: &'a [u64; METRIC_CAPACITY],
	kind: MetricKind,
}

impl Serialize for Values<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut map = serializer.serialize_map(None)?;
		for (index, (name, kind)) in self.names.names.iter().enumerate() {
			let value = self.values[index];
			if *kind == self.kind && value != ABSENT {
				map.serialize_entry(name, &milliseconds(Duration::from_nanos(value)))?;
			}
		}
		map.end()
	}
}

/// The `Statistics` struct describes the distribution of one metric over the summarized ticks, in milliseconds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Statistics {
	/// How many summarized ticks measured the metric.
	pub count: u64,
	pub mean: f64,
	pub min: f64,
	pub max: f64,
	pub p50: f64,
	pub p95: f64,
	pub p99: f64,
}

impl Statistics {
	/// Computes the statistics of `values` in nanoseconds, sorting them in place. Returns `None` for no values.
	fn of(values: &mut [u64]) -> Option<Self> {
		if values.is_empty() {
			return None;
		}
		values.sort_unstable();
		let percentile = |fraction: f64| {
			let index = ((values.len() - 1) as f64 * fraction).round() as usize;
			nanoseconds_to_milliseconds(values[index])
		};
		let sum = values.iter().map(|value| *value as f64).sum::<f64>();
		Some(Self {
			count: values.len() as u64,
			mean: sum / values.len() as f64 / 1e6,
			min: nanoseconds_to_milliseconds(values[0]),
			max: nanoseconds_to_milliseconds(values[values.len() - 1]),
			p50: percentile(0.5),
			p95: percentile(0.95),
			p99: percentile(0.99),
		})
	}
}

/// The `MetricSummary` struct is the distribution of one named metric.
#[derive(Clone, Debug, Serialize)]
pub struct MetricSummary {
	pub name: Box<str>,
	pub kind: MetricKind,
	#[serde(flatten)]
	pub statistics: Statistics,
}

/// The `Summary` struct is the aggregate view of the retained ticks that `GET /metrics` returns.
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
	/// How many ticks the summary covers.
	pub ticks: u64,
	pub first_tick: Option<u64>,
	pub last_tick: Option<u64>,
	/// How many of the covered ticks could reach the screen.
	pub presented_ticks: u64,
	/// Milliseconds from application start to the first tick that could reach the screen.
	#[serde(serialize_with = "serialize_optional_milliseconds")]
	pub time_to_first_frame: Option<Duration>,
	/// The frame delta over the covered ticks, in milliseconds.
	pub delta: Option<Statistics>,
	/// Every metric at least one covered tick measured.
	pub metrics: Vec<MetricSummary>,
}

/// Converts to milliseconds through whole nanoseconds, so values print without floating-point noise.
fn milliseconds(duration: Duration) -> f64 {
	nanoseconds_to_milliseconds(duration.as_nanos() as u64)
}

fn nanoseconds_to_milliseconds(nanoseconds: u64) -> f64 {
	nanoseconds as f64 / 1e6
}

fn serialize_optional_milliseconds<S: Serializer>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error> {
	duration.map(milliseconds).serialize(serializer)
}

/// The `MetricsLayer` struct feeds the time spent in `tracing` spans into a [`Metrics`] collector.
///
/// Install it in the global subscriber, over a registry, before the application opens its first span. Every span
/// at `DEBUG` or a less verbose level becomes a CPU metric named after the span; spans with one name share a metric
/// wherever they open. Events pass through untouched.
pub struct MetricsLayer {
	metrics: Arc<Metrics>,
	/// The metric of each call site the layer has seen, or `None` for call sites it does not track.
	callsites: RwLock<HashMap<Identifier, Option<MetricId>>>,
}

thread_local! {
	/// The spans entered on this thread that have not exited, innermost last, with the time each was entered.
	static ENTERED: RefCell<SmallVec<[(Option<MetricId>, Instant); 16]>> = RefCell::new(SmallVec::new());
}

impl MetricsLayer {
	/// Creates a layer that reports into `metrics`.
	pub fn new(metrics: Arc<Metrics>) -> Self {
		Self {
			metrics,
			callsites: RwLock::new(HashMap::new()),
		}
	}

	fn tracks(metadata: &Metadata<'_>) -> bool {
		metadata.is_span() && *metadata.level() <= tracing::Level::DEBUG
	}

	/// Returns the metric of a call site, registering it on first sight.
	fn metric(&self, metadata: &'static Metadata<'static>) -> Option<MetricId> {
		if let Some(metric) = self.callsites.read().get(&metadata.callsite()) {
			return *metric;
		}
		let metric = Self::tracks(metadata)
			.then(|| self.metrics.register(MetricKind::Cpu, metadata.name()))
			.flatten();
		self.callsites.write().insert(metadata.callsite(), metric);
		metric
	}
}

impl<S> Layer<S> for MetricsLayer
where
	S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
	fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
		if Self::tracks(metadata) {
			self.metric(metadata);
			Interest::always()
		} else {
			Interest::never()
		}
	}

	fn enabled(&self, metadata: &Metadata<'_>, _: Context<'_, S>) -> bool {
		Self::tracks(metadata)
	}

	fn on_new_span(&self, _: &Attributes<'_>, _: &Id, _: Context<'_, S>) {}

	fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
		let metric = ctx.metadata(id).and_then(|metadata| self.metric(metadata));
		ENTERED.with_borrow_mut(|entered| entered.push((metric, Instant::now())));
	}

	fn on_exit(&self, _: &Id, _: Context<'_, S>) {
		let Some((metric, entered_at)) = ENTERED.with_borrow_mut(|entered| entered.pop()) else {
			return;
		};
		if let Some(metric) = metric {
			self.metrics.add_cpu(metric, entered_at.elapsed());
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{sync::Arc, time::Duration};

	use tracing_subscriber::layer::SubscriberExt as _;

	use super::{METRIC_CAPACITY, MetricKind, Metrics, MetricsLayer, TICK_CAPACITY, TickClock};

	fn clock(tick: u64, presented: bool) -> TickClock {
		TickClock {
			tick,
			frame: Some(tick),
			time: Duration::from_millis(tick * 16),
			delta: Duration::from_micros(16_667),
			presented,
		}
	}

	#[test]
	fn ticks_carry_cpu_time_and_adopt_gpu_time_of_their_frame() {
		let metrics = Metrics::new();
		let update = metrics.register(MetricKind::Cpu, "update").unwrap();
		let bloom = metrics.register(MetricKind::Gpu, "pass.Bloom").unwrap();
		assert_eq!(metrics.register(MetricKind::Cpu, "update"), Some(update));
		assert_eq!(metrics.register(MetricKind::Gpu, "update"), None);

		metrics.add_cpu(update, Duration::from_millis(2));
		metrics.add_cpu(update, Duration::from_millis(1));
		metrics.end_tick(clock(0, false));
		metrics.end_tick(clock(1, true));
		metrics.set_gpu(0, bloom, Duration::from_micros(400));
		metrics.set_gpu(7, bloom, Duration::from_micros(400));

		let rows = metrics.ticks_since(None);
		assert_eq!(rows.len(), 2);
		assert_eq!(rows[0].tick().value(update), Some(Duration::from_millis(3)));
		assert_eq!(rows[0].tick().value(bloom), Some(Duration::from_micros(400)));
		assert_eq!(rows[1].tick().value(update), Some(Duration::ZERO));
		assert_eq!(rows[1].tick().value(bloom), None);
		assert_eq!(metrics.ticks_since(Some(0)).len(), 1);

		let json = serde_json::to_value(&rows[0]).unwrap();
		assert_eq!(json["tick"], 0);
		assert_eq!(json["frame"], 0);
		assert_eq!(json["presented"], false);
		assert_eq!(json["cpu"]["update"], 3.0);
		assert_eq!(json["gpu"]["pass.Bloom"], 0.4);
		assert!(json["gpu"].get("update").is_none());
	}

	#[test]
	fn summary_reports_percentiles_over_the_requested_ticks() {
		let metrics = Metrics::new();
		let tick = metrics.register(MetricKind::Cpu, "tick").unwrap();
		for index in 0..10 {
			metrics.add_cpu(tick, Duration::from_millis(index + 1));
			metrics.end_tick(clock(index, index >= 5));
		}

		let summary = metrics.summary(None, false);
		assert_eq!((summary.ticks, summary.presented_ticks), (10, 5));
		assert_eq!((summary.first_tick, summary.last_tick), (Some(0), Some(9)));
		assert_eq!(summary.time_to_first_frame, Some(Duration::from_millis(80)));
		let statistics = &summary.metrics[0].statistics;
		assert_eq!((statistics.count, statistics.min, statistics.max), (10, 1.0, 10.0));
		assert_eq!((statistics.mean, statistics.p50, statistics.p99), (5.5, 6.0, 10.0));

		let presented = metrics.summary(Some(6), true);
		assert_eq!(presented.ticks, 3);
		assert_eq!(presented.metrics[0].statistics.min, 8.0);
		assert_eq!(presented.delta.as_ref().unwrap().count, 3);
	}

	#[test]
	fn the_ring_keeps_the_newest_ticks_and_the_capacity_of_metrics_is_bounded() {
		let metrics = Metrics::new();
		for index in 0..TICK_CAPACITY as u64 + 3 {
			metrics.end_tick(clock(index, true));
		}
		let rows = metrics.ticks_since(None);
		assert_eq!(rows.len(), TICK_CAPACITY);
		assert_eq!(rows[0].tick().tick, 3);
		assert_eq!(rows[TICK_CAPACITY - 1].tick().tick, TICK_CAPACITY as u64 + 2);

		for index in 0..METRIC_CAPACITY {
			assert!(metrics.register(MetricKind::Cpu, &format!("metric-{index}")).is_some());
		}
		assert_eq!(metrics.register(MetricKind::Cpu, "one too many"), None);
	}

	#[test]
	fn the_layer_attributes_span_time_to_the_tick_it_ran_in() {
		let metrics = Arc::new(Metrics::new());
		let subscriber = tracing_subscriber::registry().with(MetricsLayer::new(Arc::clone(&metrics)));
		tracing::subscriber::with_default(subscriber, || {
			{
				let span = tracing::debug_span!("metrics-test-outer");
				let _enter = span.enter();
				for _ in 0..2 {
					let span = tracing::debug_span!("metrics-test-inner");
					let _enter = span.enter();
					std::thread::sleep(Duration::from_millis(2));
				}
				let span = tracing::trace_span!("metrics-test-ignored");
				let _enter = span.enter();
			}
			metrics.end_tick(clock(0, true));
			metrics.end_tick(clock(1, true));
		});

		let rows = metrics.ticks_since(None);
		let outer = metrics.register(MetricKind::Cpu, "metrics-test-outer").unwrap();
		let inner = metrics.register(MetricKind::Cpu, "metrics-test-inner").unwrap();
		assert!(rows[0].tick().value(inner).unwrap() >= Duration::from_millis(4));
		assert!(rows[0].tick().value(outer).unwrap() >= rows[0].tick().value(inner).unwrap());
		assert_eq!(rows[1].tick().value(inner), Some(Duration::ZERO));
		// Trace-level spans are too fine-grained for per-tick metrics, so the layer never registers them.
		assert!(
			metrics
				.summary(None, false)
				.metrics
				.iter()
				.all(|metric| &*metric.name != "metrics-test-ignored")
		);
	}
}
