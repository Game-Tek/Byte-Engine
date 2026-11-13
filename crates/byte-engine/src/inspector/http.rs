use std::{
	io::{self, Write as _},
	net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
	time::Duration,
};

use oxhttp::{
	ListeningServer, Server,
	model::{Body, Method, Request, Response, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::application::parameters::Parameters;
use crate::{
	application::LoopWaker,
	configuration::ConfigurationValue,
	core::{EntityHandle, factory::Handle},
	inspector::{
		DefaultInspector, EventCursor, EventFilter, EventWaitError, MAX_EVENT_WAIT, MAX_SCREENSHOT_CAPTURES, ScreenshotCapture,
		ScreenshotError, ScreenshotFormat, ScreenshotSelection, ScreenshotSubmitError,
	},
};

const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// The `HttpInspectorServer` struct exposes the Byte Engine Inspection Protocol
/// through an HTTP API.
///
/// Clients use this server to inspect factory-created entities and drain
/// passive publication ranges. `GET /entities` returns each numeric `target`,
/// its Rust `types`, and a `properties` object with every tracked value it has:
/// its `name` and its latest `position` as `[x, y, z]`. Filter entities with an
/// exact `name`, `type`, or both. `GET /messages` returns the `scope`, complete
/// generic `type`, `first_sequence`, and `count` for each route that published
/// since the previous request. Payloads remain opaque. `GET /messages/types`
/// returns each registered protocol `type` and the reflected shape of its JSON
/// `payload` so editor clients can build controls before posting a message.
///
/// `POST /messages` accepts message types registered through
/// [`DefaultInspector::register_message`]. To move an entity, send a JSON object with
/// `type: "TransformationUpdate"`, its numeric `target` handle, and the complete
/// reflected [`Transform`](crate::gameplay::Transform) payload. To remove an
/// entity, send `type: "Delete"` or `type: "Destroy"` with its target and a
/// reflected unit payload represented by JSON `null`.
/// To drive a named action created through
/// [`GraphicsApplication::world`](crate::application::graphics::GraphicsApplication::world),
/// resolve its target with `GET /entities?name=<action>` and send `type:
/// "TriggerAction"` with a reflected [`Value`](crate::input::Value).
///
/// `POST /configuration` changes a runtime setting while the application runs. Send a JSON object with the full
/// `parameter` name, such as `render.lod.force`, and its `value`; the response is the update's event, and
/// `GET /configuration` or the `configuration` events of `GET /events` report when the owning system set or refused
/// it.
///
/// The server shares the [`DefaultInspector`] handle, so another transport can
/// serve a second protocol surface from the same handle.
/// `GET /screenshots?sink=<index>` returns one image. Add `target=<name>` to read
/// a scene target, `pass=<name>` to read it right after that pass, and
/// `previous=true` to read the copy of a history target that the previous frame
/// wrote. Add `format=exr` for a lossless HDR image or `format=raw` for the GPU
/// bytes. To debug several targets from the same frame, `POST /screenshots` with
/// a JSON `captures` list; the response is `multipart/form-data` with one part
/// per capture.
///
/// `GET /metrics` summarizes the CPU time of every engine span and the GPU time of
/// every scene pipeline, render pass, and stage timed inside them over the retained ticks, in milliseconds,
/// with `count`, `mean`, `min`, `max`, `p50`, `p95`, and `p99`. Add
/// `since=<tick>` to summarize only later ticks and `presented=true` to count only
/// ticks that could reach the screen. `GET /metrics/frames` streams the retained
/// ticks themselves as NDJSON, one object per line with `tick`, `frame`, `time`,
/// `dt`, `presented`, and `cpu` and `gpu` maps; it takes the same filters plus
/// `limit=<count>` to keep only the newest rows. GPU values of the last frames in
/// flight arrive once those frames complete.
///
/// `GET /events` reports what changed since a client's cursor: ended ticks, GPU
/// times of completed frames, configuration changes, and the application closing.
/// Call it without `after` to get a cursor, then pass each response's `cursor` as
/// `after` with `wait=<milliseconds>` to wait for the next events. It doesn't wake
/// an idle application.
///
/// See the [inspector reference](/docs/reference/inspector) for its endpoints, payloads, and limits.
pub struct HttpInspectorServer {
	_server: ListeningServer,
}

/// The port the inspector serves on unless the `inspector.port` parameter names another or another application
/// already uses it.
pub const DEFAULT_PORT: u16 = 6680;

/// The addresses the inspector serves on unless the `inspector.address` parameter names another, so only this machine
/// reaches it.
const LOOPBACK: [IpAddr; 2] = [IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)];

impl HttpInspectorServer {
	/// Starts the HTTP inspector transport, on the loopback interface unless `address` names another.
	///
	/// Next, request `GET /entities` to verify that the application is available.
	///
	/// # Parameters
	/// Read from `parameters`, the `inspector` subtree of the application's:
	/// - `address`: The IPv4 or IPv6 address to serve on, such as `0.0.0.0` or `::` to reach the inspector from
	///   another machine. Defaults to both loopback addresses. An address that is not loopback is logged as a
	///   warning, because the inspector has no authentication. A value that is not an address stops startup.
	/// - `port`: The port to serve on. Defaults to [`DEFAULT_PORT`], or to a free port the system picks when that one
	///   is in use, which is logged as a warning. A named port that is in use, or a value that is not a port, stops
	///   startup.
	pub fn new(inspector: EntityHandle<DefaultInspector>, waker: LoopWaker, parameters: impl Parameters) -> Self {
		let address: Option<IpAddr> = parameters
			.get_parameter("address")
			.map(|address| address.parse().unwrap_or_else(|error| panic!("{error}")));
		let addresses = match &address {
			Some(address) => std::slice::from_ref(address),
			None => &LOOPBACK,
		};
		if let Some(address) = addresses.iter().find(|address| !address.is_loopback()) {
			log::warn!(
				"HTTP inspector serves on {address}, which other machines can reach. The inspector has no authentication, so anyone who reaches it can read the application, send it messages, or close it. Leave `inspector.address` unset to serve only this machine."
			);
		}
		let port = match parameters.get_parameter("port") {
			Some(port) => port.parse().unwrap_or_else(|error| panic!("{error}")),
			None => free_port(addresses, DEFAULT_PORT).unwrap_or_else(|error| {
				panic!(
					"HTTP inspector could not find a free port. The most likely cause is that no network interface has the inspector's address, or it does not accept listeners: {error}"
				)
			}),
		};
		Self::spawn(inspector, waker, addresses.iter().map(|&address| SocketAddr::new(address, port)))
		.unwrap_or_else(|error| {
			panic!(
				"HTTP inspector could not start. The most likely cause is that port {port} is already in use, for example by another running application; give one of them a different `inspector.port`: {error}"
			)
		})
	}

	/// Starts the inspector on each requested socket address.
	fn spawn(
		inspector: EntityHandle<DefaultInspector>,
		waker: LoopWaker,
		addresses: impl IntoIterator<Item = SocketAddr>,
	) -> io::Result<Self> {
		let mut server = Server::new(move |request| {
			// Waiting for events only reads; waking the loop for it would make every tick end a wait and start the
			// next tick, so an idle application would never idle.
			let reads_events = request.method() == Method::GET && request.uri().path() == "/events";
			let response = handle_request(&*inspector, &waker, request);
			// A request is input to the application, like a window event, so an idle loop runs after it.
			if !reads_events {
				waker.wake();
			}
			response
		});

		for address in addresses {
			server = server.bind(address);
		}
		server = server.with_global_timeout(Duration::from_secs(10));
		server = server.with_max_concurrent_connections(8);

		let server = server.spawn()?;

		Ok(Self { _server: server })
	}
}

/// Returns `preferred` when every address in `addresses` can listen on it, otherwise a free port the system picks for
/// all of them, logged as a warning.
///
/// Ports are probed and released before the server binds them, because a server that binds one address and fails on
/// another keeps listening on the first.
fn free_port(addresses: &[IpAddr], preferred: u16) -> io::Result<u16> {
	// Port 0 lets the system pick for the first address; the later addresses then probe that same port.
	let bind_all = |port| {
		addresses.iter().try_fold(port, |port, &address| {
			TcpListener::bind((address, port))?.local_addr().map(|a| a.port())
		})
	};
	bind_all(preferred).or_else(|_| {
		// The system picks a port that is free on the first address only, so pick again while a later one uses it.
		let port = (0..8).fold(bind_all(0), |port, _| port.or_else(|_| bind_all(0)))?;
		log::warn!("HTTP inspector port {preferred} is in use, so the inspector serves on port {port} instead.");
		Ok(port)
	})
}

/// Answers one inspector request.
fn handle_request(inspector: &DefaultInspector, waker: &LoopWaker, request: &mut Request<Body>) -> Response<Body> {
	match (request.method(), request.uri().path()) {
		(&Method::GET, "/screenshots") => screenshot_response(inspector, waker, request.uri().query()),
		(&Method::POST, "/screenshots") => screenshots_response(inspector, waker, request.body_mut()),
		(&Method::GET, "/messages") => messages_response(inspector),
		(&Method::GET, "/messages/types") => message_types_response(inspector),
		(&Method::POST, "/messages") => message_response(inspector, request.body_mut()),
		(&Method::GET, "/configuration") => json_response(&inspector.configuration_events()),
		(&Method::POST, "/configuration") => configuration_response(inspector, request.body_mut()),
		#[cfg(feature = "headed")]
		(&Method::GET, "/debug-views") => json_response(&inspector.debug_views()),
		(&Method::GET, "/metrics") => metrics_response(inspector, request.uri().query()),
		(&Method::GET, "/metrics/frames") => metrics_frames_response(inspector, request.uri().query()),
		(&Method::GET, "/entities") => entities_response(inspector, request.uri().query()),
		(&Method::GET, "/events") => events_response(inspector, request.uri().query()),
		(&Method::DELETE, "/") => {
			inspector.close_application();
			response(StatusCode::OK, Body::empty())
		}
		_ => response(StatusCode::NOT_FOUND, Body::empty()),
	}
}

/// Serializes the current factory-backed entity catalog.
fn entities_response(inspector: &DefaultInspector, query: Option<&str>) -> Response<Body> {
	let query = match parse_entity_query(query) {
		Ok(query) => query,
		Err(()) => {
			return response(
				StatusCode::BAD_REQUEST,
				"Entity query is malformed. The most likely cause is an unknown, duplicate, empty, or invalidly encoded `type`, `class`, or `name` parameter.",
			);
		}
	};
	json_response(&inspector.entities(query.entity_type.as_deref(), query.name.as_deref()))
}

#[derive(Debug, Default, PartialEq, Eq)]
/// The `EntityQuery` struct contains the exact-match filters accepted by the entity endpoint.
struct EntityQuery {
	entity_type: Option<String>,
	name: Option<String>,
}

/// Parses optional entity filters without accepting unknown or duplicate parameters.
fn parse_entity_query(query: Option<&str>) -> Result<EntityQuery, ()> {
	let Some(query) = query else {
		return Ok(EntityQuery::default());
	};
	let [entity_type, name] = parse_query(query, [&["type", "class"], &["name"]])?;
	Ok(EntityQuery { entity_type, name })
}

/// Drains and serializes passive message publications without payloads.
fn messages_response(inspector: &DefaultInspector) -> Response<Body> {
	let messages = inspector.drain_messages();
	json_response(&serde_json::json!({ "messages": messages }))
}

/// Serializes the protocol message types accepted by the posting endpoint.
fn message_types_response(inspector: &DefaultInspector) -> Response<Body> {
	let types = inspector.message_types();
	json_response(&serde_json::json!({ "types": types }))
}

/// The filters the metrics endpoints share.
struct MetricsQuery {
	/// Only ticks after this one.
	since: Option<u64>,
	/// Only ticks that could reach the screen.
	presented: bool,
	/// At most this many of the newest rows.
	limit: Option<usize>,
}

/// Parses `since`, `presented`, and `limit`, rejecting unknown fields and malformed numbers.
fn parse_metrics_query(query: Option<&str>) -> Result<MetricsQuery, ()> {
	let Some(query) = query else {
		return Ok(MetricsQuery {
			since: None,
			presented: false,
			limit: None,
		});
	};
	let [since, presented, limit] = parse_query(query, [&["since"], &["presented"], &["limit"]])?;
	let presented = match presented.as_deref() {
		None | Some("false") => false,
		Some("true") => true,
		Some(_) => return Err(()),
	};
	Ok(MetricsQuery {
		since: since.map(|since| since.parse()).transpose().map_err(|_| ())?,
		presented,
		limit: limit.map(|limit| limit.parse()).transpose().map_err(|_| ())?,
	})
}

const METRICS_QUERY_ERROR: &str = "Metrics query is invalid. The most likely cause is a field other than `since`, `presented`, or `limit`, or a value that is not a number or `true`/`false`.";

/// Summarizes the retained ticks as JSON.
fn metrics_response(inspector: &DefaultInspector, query: Option<&str>) -> Response<Body> {
	let Ok(query) = parse_metrics_query(query) else {
		return response(StatusCode::BAD_REQUEST, METRICS_QUERY_ERROR);
	};
	json_response(&inspector.metrics().summary(query.since, query.presented))
}

/// Streams the retained ticks as NDJSON, oldest first.
fn metrics_frames_response(inspector: &DefaultInspector, query: Option<&str>) -> Response<Body> {
	let Ok(query) = parse_metrics_query(query) else {
		return response(StatusCode::BAD_REQUEST, METRICS_QUERY_ERROR);
	};
	let rows = inspector.metrics().ticks_since(query.since);
	let rows = rows.iter().filter(|row| !query.presented || row.tick().presented);
	let rows = match query.limit {
		Some(limit) => rows
			.rev()
			.take(limit)
			.collect::<Vec<_>>()
			.into_iter()
			.rev()
			.collect::<Vec<_>>(),
		None => rows.collect(),
	};
	let mut body = Vec::new();
	for row in rows {
		if let Err(error) = serde_json::to_writer(&mut body, row) {
			return response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("Metrics row could not be serialized. The most likely cause is an unsupported value: {error}"),
			);
		}
		body.push(b'\n');
	}
	Response::builder()
		.header("Content-Type", "application/x-ndjson")
		.body(Body::from(body))
		.expect("Inspector NDJSON response is valid. The most likely cause is an invalid static header name.")
}

const EVENTS_QUERY_ERROR: &str = "Events query is invalid. The most likely cause is a field other than `after`, `wait`, or `events`, a cursor that no response returned, a `wait` that isn't a number of milliseconds up to 30000, or an event other than `tick`, `gpu`, or `configuration`. Pass the `cursor` of the previous response unchanged.";

/// Parses `after`, `wait`, and `events`, rejecting unknown fields, cursors no response returned, and waits above
/// [`MAX_EVENT_WAIT`].
fn parse_events_query(query: Option<&str>) -> Result<(Option<EventCursor>, Duration, EventFilter), ()> {
	let [after, wait, events] = match query {
		Some(query) => parse_query(query, [&["after"], &["wait"], &["events"]])?,
		None => [None, None, None],
	};
	let wait = Duration::from_millis(wait.map(|wait| wait.parse()).transpose().map_err(|_| ())?.unwrap_or(0));
	if wait > MAX_EVENT_WAIT {
		return Err(());
	}
	Ok((
		after.map(|after| after.parse()).transpose()?,
		wait,
		events.map(|events| events.parse()).transpose()?.unwrap_or_default(),
	))
}

/// Returns the events after the `after` cursor, waiting up to `wait` milliseconds for one.
fn events_response(inspector: &DefaultInspector, query: Option<&str>) -> Response<Body> {
	let Ok((after, wait, filter)) = parse_events_query(query) else {
		return response(StatusCode::BAD_REQUEST, EVENTS_QUERY_ERROR);
	};
	match inspector.wait_for_events(after, filter, wait) {
		Ok(batch) => json_response(&batch),
		Err(EventWaitError::Busy) => response(
			StatusCode::SERVICE_UNAVAILABLE,
			"Events request could not wait. The most likely cause is that four other requests already wait for events. Retry without `wait`, or after one of them returns.",
		),
	}
}

fn json_response(value: &impl Serialize) -> Response<Body> {
	match serde_json::to_vec(value) {
		Ok(body) => Response::builder()
			.header("Content-Type", "application/json")
			.body(Body::from(body))
			.expect("Inspector JSON response is valid. The most likely cause is an invalid static header name."),
		Err(error) => response(
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("Inspector response could not be serialized. The most likely cause is an unsupported value: {error}"),
		),
	}
}

/// The `MessageRequest` struct defines the complete message envelope accepted over HTTP.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageRequest {
	#[serde(rename = "type")]
	message_type: String,
	target: u32,
	/// The payload text, which the registered message parses once against its reflected shape.
	payload: Box<serde_json::value::RawValue>,
}

/// Parses and posts one complete message envelope from the HTTP request body.
fn message_response(inspector: &DefaultInspector, body: &mut Body) -> Response<Body> {
	let request: MessageRequest = match serde_json::from_reader(body) {
		Ok(request) => request,
		Err(error) => {
			return response(
				StatusCode::BAD_REQUEST,
				format!(
					"Inspector message request is invalid. The most likely cause is a malformed or incomplete request body: {error}"
				),
			);
		}
	};
	if request.message_type.is_empty() {
		return response(
			StatusCode::BAD_REQUEST,
			"Inspector message request is invalid. The most likely cause is an empty `type`.",
		);
	}
	if request.target == 0 {
		return response(
			StatusCode::BAD_REQUEST,
			"Inspector message target is invalid. Zero is reserved for absent entities. Use a target returned by the entities endpoint.",
		);
	}
	match inspector.post_message(&request.message_type, Handle::from_id(request.target), request.payload.get()) {
		Ok(()) => response(StatusCode::NO_CONTENT, Body::empty()),
		Err(error) => response(StatusCode::BAD_REQUEST, error),
	}
}

/// The `ConfigurationRequest` struct is the body of `POST /configuration`: one startup parameter to change while the
/// application runs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationRequest {
	/// The full parameter name, as on the command line without the leading dashes, such as `render.lod.force`.
	parameter: String,
	value: ConfigurationValue,
}

/// Queues one configuration update and answers with its event, which starts pending unless no system owns the
/// parameter.
fn configuration_response(inspector: &DefaultInspector, body: &mut Body) -> Response<Body> {
	let request: ConfigurationRequest = match serde_json::from_reader(body) {
		Ok(request) => request,
		Err(error) => {
			return response(
				StatusCode::BAD_REQUEST,
				format!(
					"Configuration request is invalid. The most likely cause is a body other than `{{\"parameter\": <name>, \"value\": <boolean, number, or text>}}`: {error}"
				),
			);
		}
	};
	let event = inspector.update_configuration(&request.parameter, request.value);
	match json_response(&event) {
		mut accepted if accepted.status() == StatusCode::OK => {
			*accepted.status_mut() = StatusCode::ACCEPTED;
			accepted
		}
		failed => failed,
	}
}

/// The `CaptureRequest` struct pairs one capture with the encoding its client asked for.
#[derive(Debug, PartialEq, Eq)]
struct CaptureRequest {
	selection: ScreenshotSelection,
	format: ScreenshotFormat,
}

/// The `EncodedCapture` struct keeps an encoded image with the texture layout that its HTTP headers describe.
struct EncodedCapture {
	format: ghi::Formats,
	extent: utils::Extent,
	bytes_per_row: usize,
	image: Vec<u8>,
}

/// Handles one single-image screenshot request after HTTP routing has selected the endpoint.
fn screenshot_response(inspector: &DefaultInspector, waker: &LoopWaker, query: Option<&str>) -> Response<Body> {
	let Ok(capture) = parse_screenshot_query(query) else {
		return response(
			StatusCode::BAD_REQUEST,
			"Screenshot query is malformed. The most likely cause is a missing sink, an unknown or duplicate parameter, `pass` without `target`, `previous` with `pass`, or an unknown `format`.",
		);
	};
	let (frame, mut encoded) = match capture_screenshots(inspector, waker, std::slice::from_ref(&capture)) {
		Ok(captures) => captures,
		Err((status, message)) => return response(status, message),
	};
	let encoded = encoded
		.pop()
		.expect("A completed screenshot request has one result per capture.");

	let mut builder = Response::builder()
		.status(StatusCode::OK)
		.header("X-Byte-Engine-Frame", frame.to_string());
	for (name, value) in capture_headers(&capture, &encoded) {
		builder = builder.header(name, value);
	}
	builder
		.body(Body::from(encoded.image))
		.expect("Screenshot HTTP response is valid. The most likely cause of failure is an invalid static header name.")
}

/// The `ScreenshotsBody` struct defines the JSON body of a same-frame screenshot batch.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenshotsBody {
	captures: Vec<CaptureFields>,
}

/// The `CaptureFields` struct holds the capture fields shared by the single-image query and each batch capture.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureFields {
	sink: usize,
	pass: Option<String>,
	target: Option<String>,
	#[serde(default)]
	previous: bool,
	format: Option<String>,
}

impl CaptureFields {
	/// Resolves the fields into a capture, rejecting empty names and combinations that select nothing.
	fn resolve(self) -> Result<CaptureRequest, ()> {
		if self.pass.as_deref() == Some("") || self.target.as_deref() == Some("") {
			return Err(());
		}
		let capture = match (self.pass, self.target, self.previous) {
			(None, None, false) => ScreenshotCapture::FinalSwapchain,
			(Some(pass), Some(target), false) => ScreenshotCapture::AfterPass { pass, target },
			(None, Some(target), false) => ScreenshotCapture::SceneTarget { target },
			(None, Some(target), true) => ScreenshotCapture::PreviousSceneTarget { target },
			_ => return Err(()),
		};
		let format = match self.format.as_deref() {
			None | Some("png") => ScreenshotFormat::Png,
			Some("exr") => ScreenshotFormat::Exr,
			Some("raw") => ScreenshotFormat::Raw,
			Some(_) => return Err(()),
		};
		Ok(CaptureRequest {
			selection: ScreenshotSelection {
				sink: self.sink,
				capture,
			},
			format,
		})
	}
}

/// Handles one same-frame screenshot batch and returns every image as one `multipart/form-data` part.
fn screenshots_response(inspector: &DefaultInspector, waker: &LoopWaker, body: &mut Body) -> Response<Body> {
	let captures = serde_json::from_reader::<_, ScreenshotsBody>(body)
		.map_err(|error| error.to_string())
		.and_then(|body| {
			body.captures
				.into_iter()
				.enumerate()
				.map(|(index, capture)| {
					capture.resolve().map_err(|()| {
						format!(
							"capture {index} has an empty name, `pass` without `target`, `previous` with `pass`, or an unknown `format`"
						)
					})
				})
				.collect::<Result<Vec<_>, _>>()
		});
	let captures = match captures {
		Ok(captures) => captures,
		Err(error) => {
			return response(
				StatusCode::BAD_REQUEST,
				format!(
					"Screenshot request is invalid. The most likely cause is a malformed body or an invalid capture: {error}"
				),
			);
		}
	};
	let (frame, encoded) = match capture_screenshots(inspector, waker, &captures) {
		Ok(captures) => captures,
		Err((status, message)) => return response(status, message),
	};

	let boundary = multipart_boundary(frame, &encoded);
	let mut body = Vec::with_capacity(encoded.iter().map(|capture| capture.image.len() + 512).sum());
	for (index, (capture, encoded)) in captures.iter().zip(encoded).enumerate() {
		// Writing into a vector cannot fail.
		let _ = write!(
			body,
			"--{boundary}\r\nContent-Disposition: form-data; name=\"{index}\"; filename=\"{}\"\r\n",
			capture_file_name(index, capture)
		);
		for (name, value) in capture_headers(capture, &encoded) {
			let _ = write!(body, "{name}: {value}\r\n");
		}
		body.extend_from_slice(b"\r\n");
		body.extend_from_slice(&encoded.image);
		body.extend_from_slice(b"\r\n");
	}
	let _ = write!(body, "--{boundary}--\r\n");

	Response::builder()
		.status(StatusCode::OK)
		.header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
		.header("X-Byte-Engine-Frame", frame.to_string())
		.body(Body::from(body))
		.expect("Screenshot HTTP response is valid. The most likely cause of failure is an invalid static header name.")
}

/// Captures every request in one frame and encodes each image, or returns the HTTP status and message of the first
/// failure.
///
/// Encoding runs on this transport thread so HDR encoders never delay a graphics frame.
fn capture_screenshots(
	inspector: &DefaultInspector,
	waker: &LoopWaker,
	captures: &[CaptureRequest],
) -> Result<(u64, Vec<EncodedCapture>), (StatusCode, String)> {
	let selections = captures.iter().map(|capture| capture.selection.clone()).collect();
	let response_receiver = match inspector.request_screenshots(selections) {
		// Only a rendered frame answers the request, so the loop must run before this thread waits for it.
		Ok(receiver) => {
			waker.wake();
			receiver
		}
		Err(ScreenshotSubmitError::QueueFull) => {
			return Err((
				StatusCode::TOO_MANY_REQUESTS,
				"Screenshot queue is full. The most likely cause is that capture requests arrive faster than graphics frames can complete them.".to_string(),
			));
		}
		Err(ScreenshotSubmitError::CaptureCount) => {
			return Err((
				StatusCode::BAD_REQUEST,
				format!(
					"Screenshot request has an unsupported number of captures. The most likely cause is an empty `captures` list or more than {MAX_SCREENSHOT_CAPTURES} captures."
				),
			));
		}
	};
	let Ok(screenshots) = response_receiver.recv_timeout(SCREENSHOT_TIMEOUT) else {
		return Err((
			StatusCode::GATEWAY_TIMEOUT,
			"Screenshot request timed out. The most likely cause is that the graphics thread did not complete a frame before the deadline.".to_string(),
		));
	};

	// Name the failing capture only in a batch, where the client cannot otherwise tell which one failed.
	let batch = captures.len() > 1;
	let encoded = captures
		.iter()
		.zip(screenshots.captures)
		.enumerate()
		.map(|(index, (capture, result))| {
			result
				.map_err(screenshot_error)
				.and_then(|readback| {
					let (format, extent, bytes_per_row) = (readback.format, readback.extent, readback.bytes_per_row);
					let image = capture
						.format
						.encode(readback)
						.map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
					Ok(EncodedCapture {
						format,
						extent,
						bytes_per_row,
						image,
					})
				})
				.map_err(|(status, message)| {
					(
						status,
						if batch {
							format!("Capture {index}: {message}")
						} else {
							message
						},
					)
				})
		})
		.collect::<Result<Vec<_>, _>>()?;
	Ok((screenshots.frame, encoded))
}

/// Maps a capture failure to its HTTP status and client message.
fn screenshot_error(error: ScreenshotError) -> (StatusCode, String) {
	let (status, message) = match error {
		ScreenshotError::SinkNotFound => (
			StatusCode::NOT_FOUND,
			"Screenshot sink was not found. The most likely cause is that the sink index does not identify a renderer window.",
		),
		ScreenshotError::SinkUnavailable => (
			StatusCode::CONFLICT,
			"Screenshot sink is unavailable. The most likely cause is that its swapchain image could not be acquired for this frame.",
		),
		ScreenshotError::PassNotFound => (
			StatusCode::NOT_FOUND,
			"Screenshot render pass was not found. The most likely cause is that the selected sink has no pass with the requested name.",
		),
		ScreenshotError::PassAmbiguous => (
			StatusCode::CONFLICT,
			"Screenshot render pass is ambiguous. The most likely cause is that the selected sink has multiple passes with the requested name.",
		),
		ScreenshotError::TargetNotWritten => (
			StatusCode::NOT_FOUND,
			"Screenshot target was not written by the render pass. The most likely cause is that the target name is missing, read-only, or belongs to another pass.",
		),
		ScreenshotError::TargetHasNoHistory => (
			StatusCode::NOT_FOUND,
			"Screenshot target has no previous frame. The most likely cause is that the target was not created as a history target, so each frame overwrites it.",
		),
		ScreenshotError::Internal(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error),
	};
	(status, message.to_string())
}

/// Returns the HTTP headers that describe one encoded capture.
///
/// Raw images also report their row pitch, which can include GPU padding.
fn capture_headers(capture: &CaptureRequest, encoded: &EncodedCapture) -> impl Iterator<Item = (&'static str, String)> {
	[
		Some(("Content-Type", capture.format.content_type().to_string())),
		Some(("X-Byte-Engine-Sink", capture.selection.sink.to_string())),
		Some(("X-Byte-Engine-Format", format!("{:?}", encoded.format))),
		Some(("X-Byte-Engine-Width", encoded.extent.width().to_string())),
		Some(("X-Byte-Engine-Height", encoded.extent.height().to_string())),
		(capture.format == ScreenshotFormat::Raw).then(|| ("X-Byte-Engine-Bytes-Per-Row", encoded.bytes_per_row.to_string())),
	]
	.into_iter()
	.flatten()
}

/// Names one batch part after its capture, so clients that save parts as files can tell them apart.
fn capture_file_name(index: usize, capture: &CaptureRequest) -> String {
	let sink = capture.selection.sink;
	let label = match &capture.selection.capture {
		ScreenshotCapture::FinalSwapchain => "swapchain".to_string(),
		ScreenshotCapture::AfterPass { pass, target } => format!("{pass}-{target}"),
		ScreenshotCapture::SceneTarget { target } => target.clone(),
		ScreenshotCapture::PreviousSceneTarget { target } => format!("{target}-previous"),
	};
	// Keep only characters that are safe in a quoted header value and in file names on every platform.
	let label = label
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
				character
			} else {
				'_'
			}
		})
		.collect::<String>();
	format!("{index}-sink{sink}-{label}.{}", capture.format.extension())
}

/// Returns a multipart boundary that no encoded image contains, so no part can end early.
fn multipart_boundary(frame: u64, captures: &[EncodedCapture]) -> String {
	(0u32..)
		.map(|attempt| format!("byte-engine-frame-{frame}-{attempt}"))
		.find(|boundary| {
			let boundary = boundary.as_bytes();
			captures
				.iter()
				.all(|capture| !capture.image.windows(boundary.len()).any(|window| window == boundary))
		})
		.expect("An unbounded attempt counter always finds a boundary absent from finite images.")
}

/// Parses the complete screenshot query without accepting unknown or duplicate parameters.
fn parse_screenshot_query(query: Option<&str>) -> Result<CaptureRequest, ()> {
	let [sink, pass, target, previous, format] = parse_query(
		query.ok_or(())?,
		[&["sink"], &["pass"], &["target"], &["previous"], &["format"]],
	)?;
	let previous = match previous.as_deref() {
		None | Some("false") => false,
		Some("true") => true,
		Some(_) => return Err(()),
	};
	CaptureFields {
		sink: sink.ok_or(())?.parse().map_err(|_| ())?,
		pass,
		target,
		previous,
		format,
	}
	.resolve()
}

/// Parses and decodes a fixed set of query fields without duplicates.
fn parse_query<const N: usize>(query: &str, names: [&[&str]; N]) -> Result<[Option<String>; N], ()> {
	let mut values = std::array::from_fn(|_| None);
	for parameter in query.split('&') {
		let (name, value) = parameter.split_once('=').ok_or(())?;
		let value = decode_query_component(value)?;
		if value.is_empty() {
			return Err(());
		}
		let slot = names
			.iter()
			.position(|accepted| accepted.contains(&name))
			.and_then(|index| values.get_mut(index))
			.ok_or(())?;
		if slot.replace(value).is_some() {
			return Err(());
		}
	}
	Ok(values)
}

/// Decodes one URL query component and rejects incomplete escapes and non-UTF-8 bytes.
fn decode_query_component(value: &str) -> Result<String, ()> {
	let bytes = value.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		match bytes[index] {
			b'+' => decoded.push(b' '),
			b'%' => {
				let high = bytes.get(index + 1).copied().and_then(hex_value).ok_or(())?;
				let low = bytes.get(index + 2).copied().and_then(hex_value).ok_or(())?;
				decoded.push(high << 4 | low);
				index += 2;
			}
			byte => decoded.push(byte),
		}
		index += 1;
	}
	String::from_utf8(decoded).map_err(|_| ())
}

fn hex_value(byte: u8) -> Option<u8> {
	match byte {
		b'0'..=b'9' => Some(byte - b'0'),
		b'a'..=b'f' => Some(byte - b'a' + 10),
		b'A'..=b'F' => Some(byte - b'A' + 10),
		_ => None,
	}
}

fn response(status: StatusCode, body: impl Into<Body>) -> Response<Body> {
	Response::builder()
		.status(status)
		.body(body.into())
		.expect("Inspector HTTP error response is valid. The most likely cause of failure is an invalid status code.")
}

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
		sync::Arc,
		time::Duration,
	};

	use math::Point;

	use super::HttpInspectorServer;
	use crate::{
		application::{Events, Parameter},
		configuration::Configuration,
		core::{
			EntityHandle,
			channel::{Channel as _, DefaultChannel},
			factory::Handle,
			listener::{DefaultListener, Listener as _},
			message::DeleteMessage,
			message_bus::MessageBus,
		},
		gameplay::{Name, Transform, TransformationUpdate},
		inspector::{
			DESTROY_MESSAGE_TYPE, DefaultInspector, ScreenshotCapture, ScreenshotError, ScreenshotFormat, ScreenshotSelection,
			Screenshots, TRANSFORMATION_UPDATE_MESSAGE_TYPE, screenshot::ScreenshotBroker,
		},
		metrics::{MetricKind, Metrics, TickClock},
	};

	/// Creates an inspector with live future-only control and transform listeners.
	fn test_inspector(
		configuration: Configuration,
	) -> (
		EntityHandle<DefaultInspector>,
		DefaultListener<Events>,
		DefaultListener<TransformationUpdate>,
	) {
		test_inspector_with(configuration, None)
	}

	/// Builds the test inspector, connected to `debug_views` when one is given.
	fn test_inspector_with(
		configuration: Configuration,
		debug_views: Option<Arc<crate::rendering::DebugViewCatalog>>,
	) -> (
		EntityHandle<DefaultInspector>,
		DefaultListener<Events>,
		DefaultListener<TransformationUpdate>,
	) {
		let events = DefaultChannel::new();
		let event_listener = events.listener();
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("http-inspector-test-world");
		let transforms = messages.channel();
		let transform_listener = transforms.listener();
		let mut inspector = DefaultInspector::new(events, configuration, messages, Arc::new(Metrics::new()));
		inspector
			.register_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, transforms)
			.expect("register reflected transformation update");
		if let Some(debug_views) = debug_views {
			inspector.set_debug_views(debug_views);
		}
		(EntityHandle::from(inspector), event_listener, transform_listener)
	}

	/// The `TestServer` struct keeps socket setup and raw HTTP exchange out of endpoint tests.
	struct TestServer {
		_server: HttpInspectorServer,
		address: SocketAddr,
	}

	impl TestServer {
		fn new(inspector: EntityHandle<DefaultInspector>) -> Self {
			// Reserve an available local port so the test exercises the real socket path without competing for the production port.
			let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
			let address = reservation.local_addr().expect("read inspector test address");
			drop(reservation);
			let server = HttpInspectorServer::spawn(inspector, super::LoopWaker::default(), [address])
				.expect("start inspector test server");
			Self {
				_server: server,
				address,
			}
		}

		fn request(&self, method: &str, path: &str, body: &str) -> Vec<u8> {
			let mut stream = TcpStream::connect(self.address).expect("connect to inspector test server");
			stream
				.set_read_timeout(Some(Duration::from_secs(1)))
				.expect("set inspector response timeout");
			write!(
				stream,
				"{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
				body.len()
			)
			.expect("write inspector request");
			let mut response = Vec::new();
			stream.read_to_end(&mut response).expect("read inspector response");
			response
		}

		fn get_json(&self, path: &str) -> serde_json::Value {
			let response = self.request("GET", path, "");
			assert!(response.starts_with(b"HTTP/1.1 200"), "unexpected response: {response:?}");
			serde_json::from_slice(response_body(&response)).expect("parse inspector JSON response")
		}
	}

	fn response_body(response: &[u8]) -> &[u8] {
		let start = response
			.windows(4)
			.position(|window| window == b"\r\n\r\n")
			.expect("response headers")
			+ 4;
		&response[start..]
	}

	#[test]
	fn free_port_keeps_the_preferred_port_and_falls_back_when_it_is_taken() {
		let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("occupy inspector test port");
		let taken_port = taken.local_addr().expect("read occupied port").port();
		let fallback = super::free_port(&super::LOOPBACK, taken_port).expect("pick fallback port");
		assert_ne!(fallback, taken_port);
		assert_eq!(
			super::free_port(&super::LOOPBACK, fallback).expect("keep free port"),
			fallback
		);
	}

	#[test]
	fn server_serves_only_the_address_parameter() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let reservation = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let port = reservation.local_addr().expect("read inspector test address").port();
		let parameters = [
			Parameter::new("address", "127.0.0.1"),
			Parameter::new("port", &port.to_string()),
		];
		// Holding the IPv6 loopback port proves the inspector binds only the named address.
		let server = TestServer {
			_server: HttpInspectorServer::new(inspector, super::LoopWaker::default(), &parameters[..]),
			address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
		};

		assert!(server.get_json("/entities").is_array());
	}

	#[test]
	fn server_reports_registered_message_payload_shapes_over_http() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let body = TestServer::new(inspector).get_json("/messages/types");

		assert_eq!(body["types"].as_array().expect("registered message types").len(), 1);
		assert_eq!(body["types"][0]["type"], TRANSFORMATION_UPDATE_MESSAGE_TYPE);
		assert_eq!(body["types"][0]["payload"]["type"], "object");
		assert_eq!(body["types"][0]["payload"]["additional_fields"], false);
		assert_eq!(body["types"][0]["payload"]["fields"].as_array().unwrap().len(), 3);
	}

	#[test]
	fn server_reports_factory_entities_and_generic_message_publications() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("http-observation-test");
		let entity = messages.factory::<String>().create("crate".to_string());
		let generic_messages = messages.channel::<Option<u32>>();
		let _generic_listener = generic_messages.listener();
		generic_messages.send(Some(7));
		let inspector = EntityHandle::from(DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			messages,
			Arc::new(Metrics::new()),
		));
		let server = TestServer::new(inspector);
		let entities = server.get_json("/entities?type=alloc%3A%3Astring%3A%3AString");
		assert_eq!(entities[0]["target"], entity.id());
		assert_eq!(entities[0]["types"][0], std::any::type_name::<String>());

		let messages = server.get_json("/messages");
		let publications = messages["messages"].as_array().expect("publication list");
		let publication_of = |message_type: &str| {
			publications
				.iter()
				.find(|publication| publication["type"] == message_type)
				.unwrap_or_else(|| panic!("no publication of {message_type} in {publications:?}"))
		};
		let generic = publication_of(std::any::type_name::<Option<u32>>());
		assert_eq!(generic["scope"], "http-observation-test");
		assert_eq!(generic["first_sequence"], 0);
		assert_eq!(generic["count"], 1);
		// Publications are stored even before any listener exists, so the creation is reported too.
		let creation = publication_of(std::any::type_name::<crate::core::factory::CreateMessage<String>>());
		assert_eq!(creation["count"], 1);
	}

	#[test]
	fn entity_endpoint_returns_tracked_properties_and_filters_by_name() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("named-http-entity-test");
		let labels = messages.factory::<String>();
		let names = messages.factory::<Name>();
		let transforms = messages.factory::<Transform>();
		let inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			messages,
			Arc::new(Metrics::new()),
		);

		let named = labels.create("crate-model".to_string());
		names.derive(named, Name::new("shipping crate"));
		transforms.derive(named, Transform::from_position(Point::new(1.0, 2.0, 3.0)));
		let _unnamed = labels.create("barrel-model".to_string());

		let response = super::entities_response(&inspector, Some("name=shipping+crate"));
		assert_eq!(response.status(), oxhttp::model::StatusCode::OK);
		let entities: serde_json::Value = serde_json::from_reader(response.into_body()).expect("parse named entities");
		assert_eq!(entities.as_array().expect("entity array").len(), 1);
		assert_eq!(entities[0]["target"], named.id());
		assert_eq!(
			entities[0]["properties"],
			serde_json::json!({ "name": "shipping crate", "position": [1.0, 2.0, 3.0] })
		);
		assert!(
			entities[0]["types"]
				.as_array()
				.expect("entity types")
				.iter()
				.any(|entity_type| entity_type == std::any::type_name::<Name>())
		);

		let response = super::entities_response(&inspector, Some("name=crate"));
		let entities: serde_json::Value = serde_json::from_reader(response.into_body()).expect("parse exact name filter");
		assert!(entities.as_array().expect("entity array").is_empty());
	}

	#[test]
	fn server_publishes_application_close_requests() {
		let (inspector, mut events, _transforms) = test_inspector(Configuration::new());
		let response = TestServer::new(inspector).request("DELETE", "/", "");
		assert!(response.starts_with(b"HTTP/1.1 200"), "unexpected response: {response:?}");
		assert_eq!(events.read(), Some(Events::Close));
	}

	/// Answers the next screenshot request on another thread, as the graphics application would.
	fn respond_to_next_screenshot(
		screenshots: std::sync::Arc<ScreenshotBroker>,
		respond: impl FnOnce(&[ScreenshotSelection]) -> Screenshots + Send + 'static,
	) -> std::thread::JoinHandle<()> {
		std::thread::spawn(move || {
			let request = (0..100)
				.find_map(|_| {
					let request = screenshots.drain().pop();
					if request.is_none() {
						std::thread::sleep(Duration::from_millis(2));
					}
					request
				})
				.expect("receive screenshot request");
			let result = respond(&request.captures);
			request.complete(result);
		})
	}

	/// Returns a 1x1 readback that stores `bytes` in `format`.
	fn pixel(format: ghi::Formats, bytes: &[u8]) -> ghi::TextureReadback {
		ghi::TextureReadback {
			bytes: bytes.to_vec(),
			extent: utils::Extent::rectangle(1, 1),
			format,
			bytes_per_row: bytes.len(),
			bytes_per_image: bytes.len(),
		}
	}

	fn split_response(response: &[u8]) -> (&str, &[u8]) {
		let headers_end = response.len() - response_body(response).len();
		let headers = std::str::from_utf8(&response[..headers_end]).expect("UTF-8 headers");
		(headers, &response[headers_end..])
	}

	#[test]
	fn server_returns_screenshot_with_capture_headers() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let responder = respond_to_next_screenshot(inspector.screenshot_broker(), |captures| {
			assert_eq!(
				captures,
				[ScreenshotSelection {
					sink: 2,
					capture: ScreenshotCapture::FinalSwapchain,
				}]
			);
			Screenshots {
				frame: 41,
				captures: vec![Ok(pixel(ghi::Formats::BGRAu8, &[1, 2, 3, 255]))],
			}
		});
		let server = TestServer::new(inspector);

		let response = server.request("GET", "/screenshots?sink=2", "");
		responder.join().expect("join screenshot responder");

		let (headers, body) = split_response(&response);
		assert!(headers.starts_with("HTTP/1.1 200"), "unexpected response: {headers}");
		assert!(headers.contains("content-type: image/png"));
		assert!(headers.contains("x-byte-engine-frame: 41"));
		assert!(headers.contains("x-byte-engine-sink: 2"));
		assert!(headers.contains("x-byte-engine-format: BGRAu8"));
		assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"));
	}

	#[test]
	fn server_returns_raw_previous_frame_bytes_with_their_layout() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let half = half::f16::from_f32(8.0).to_bits().to_ne_bytes();
		let bytes = [half, half, half, half].concat();
		let stored = bytes.clone();
		let responder = respond_to_next_screenshot(inspector.screenshot_broker(), move |captures| {
			assert_eq!(
				captures[0].capture,
				ScreenshotCapture::PreviousSceneTarget {
					target: "Diffuse Radiance History".to_string(),
				}
			);
			Screenshots {
				frame: 7,
				captures: vec![Ok(pixel(ghi::Formats::RGBA16F, &stored))],
			}
		});
		let server = TestServer::new(inspector);

		let response = server.request(
			"GET",
			"/screenshots?sink=0&target=Diffuse+Radiance+History&previous=true&format=raw",
			"",
		);
		responder.join().expect("join screenshot responder");

		let (headers, body) = split_response(&response);
		assert!(headers.starts_with("HTTP/1.1 200"), "unexpected response: {headers}");
		assert!(headers.contains("content-type: application/octet-stream"));
		assert!(headers.contains("x-byte-engine-format: RGBA16F"));
		assert!(headers.contains("x-byte-engine-width: 1"));
		assert!(headers.contains("x-byte-engine-height: 1"));
		assert!(headers.contains("x-byte-engine-bytes-per-row: 8"));
		assert_eq!(body, bytes);
	}

	#[test]
	fn server_returns_a_same_frame_batch_as_multipart_parts() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let responder = respond_to_next_screenshot(inspector.screenshot_broker(), |captures| {
			assert_eq!(captures.len(), 2);
			assert_eq!(
				captures[1].capture,
				ScreenshotCapture::AfterPass {
					pass: "bloom".to_string(),
					target: "main".to_string(),
				}
			);
			Screenshots {
				frame: 12,
				captures: vec![
					Ok(pixel(ghi::Formats::R8UNORM, &[200])),
					Ok(pixel(ghi::Formats::RGBA8UNORM, &[1, 2, 3, 4])),
				],
			}
		});
		let server = TestServer::new(inspector);

		let response = server.request(
			"POST",
			"/screenshots",
			r#"{"captures":[{"sink":0,"target":"Sun Visibility","format":"exr"},{"sink":1,"pass":"bloom","target":"main","format":"raw"}]}"#,
		);
		responder.join().expect("join screenshot responder");

		let (headers, body) = split_response(&response);
		assert!(headers.starts_with("HTTP/1.1 200"), "unexpected response: {headers}");
		assert!(headers.contains("x-byte-engine-frame: 12"));
		let boundary = headers
			.lines()
			.find_map(|line| line.strip_prefix("content-type: multipart/form-data; boundary="))
			.expect("multipart boundary")
			.trim();

		// Each part sits between two boundary lines and splits into its headers and its image.
		let parts = split_bytes(body, format!("--{boundary}").as_bytes());
		assert_eq!(
			parts.len(),
			4,
			"expected two parts between the opening and closing boundaries"
		);
		assert_eq!(parts[3], b"--\r\n");

		let (exr_headers, exr) = split_part(parts[1]);
		assert!(exr_headers.contains(r#"name="0"; filename="0-sink0-Sun_Visibility.exr""#));
		assert!(exr_headers.contains("Content-Type: image/x-exr"));
		assert!(exr.starts_with(b"\x76\x2f\x31\x01"));

		let (raw_headers, raw) = split_part(parts[2]);
		assert!(raw_headers.contains(r#"filename="1-sink1-bloom-main.bin""#));
		assert!(raw_headers.contains("X-Byte-Engine-Sink: 1"));
		assert!(raw_headers.contains("X-Byte-Engine-Format: RGBA8UNORM"));
		assert!(raw_headers.contains("X-Byte-Engine-Bytes-Per-Row: 4"));
		assert_eq!(raw, b"\x01\x02\x03\x04\r\n");
	}

	/// Splits `bytes` at every occurrence of `separator`.
	fn split_bytes<'a>(mut bytes: &'a [u8], separator: &[u8]) -> Vec<&'a [u8]> {
		let mut parts = Vec::new();
		while let Some(position) = bytes.windows(separator.len()).position(|window| window == separator) {
			parts.push(&bytes[..position]);
			bytes = &bytes[position + separator.len()..];
		}
		parts.push(bytes);
		parts
	}

	/// Splits one multipart part at its first blank line into UTF-8 headers and binary content.
	fn split_part(part: &[u8]) -> (&str, &[u8]) {
		let end = part
			.windows(4)
			.position(|window| window == b"\r\n\r\n")
			.expect("multipart part headers");
		(
			std::str::from_utf8(&part[..end]).expect("UTF-8 part headers"),
			&part[end + 4..],
		)
	}

	#[test]
	fn batch_failure_names_the_failing_capture() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let responder = respond_to_next_screenshot(inspector.screenshot_broker(), |_| Screenshots {
			frame: 3,
			captures: vec![
				Ok(pixel(ghi::Formats::R8UNORM, &[0])),
				Err(ScreenshotError::TargetHasNoHistory),
			],
		});
		let server = TestServer::new(inspector);

		let response = server.request(
			"POST",
			"/screenshots",
			r#"{"captures":[{"sink":0},{"sink":0,"target":"Sun Visibility","previous":true}]}"#,
		);
		responder.join().expect("join screenshot responder");

		let (headers, body) = split_response(&response);
		assert!(headers.starts_with("HTTP/1.1 404"), "unexpected response: {headers}");
		assert!(body.starts_with(b"Capture 1: Screenshot target has no previous frame."));
	}

	#[test]
	fn batch_rejects_invalid_captures_before_queueing() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let server = TestServer::new(inspector);
		for body in [
			r#"{"captures":[]}"#,
			r#"{"captures":[{"sink":0,"pass":"bloom"}]}"#,
			r#"{"captures":[{"sink":0,"pass":"bloom","target":"main","previous":true}]}"#,
			r#"{"captures":[{"sink":0,"format":"tiff"}]}"#,
			r#"{"captures":[{"sink":0,"extra":1}]}"#,
		] {
			let response = server.request("POST", "/screenshots", body);
			assert!(
				response.starts_with(b"HTTP/1.1 400"),
				"unexpected response for {body}: {response:?}"
			);
		}
	}

	/// Records two ticks with CPU and GPU time, the second one presented.
	fn record_test_ticks(metrics: &Metrics) {
		let tick = metrics.register(MetricKind::Cpu, "GraphicsApplication::tick").unwrap();
		let bloom = metrics.register(MetricKind::Gpu, "pass.Bloom").unwrap();
		for (index, presented) in [(0, false), (1, true)] {
			metrics.add_cpu(tick, Duration::from_millis(2 + index));
			metrics.end_tick(TickClock {
				tick: index,
				frame: Some(index),
				time: Duration::from_millis(100 * (index + 1)),
				delta: Duration::from_micros(16_667),
				presented,
			});
		}
		metrics.set_gpu(0, bloom, Duration::from_micros(500));
	}

	#[test]
	fn server_summarizes_metrics_and_filters_presented_ticks() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		record_test_ticks(inspector.metrics());
		let server = TestServer::new(inspector);

		let summary = server.get_json("/metrics");
		assert_eq!(summary["ticks"], 2);
		assert_eq!(summary["presented_ticks"], 1);
		assert_eq!(summary["time_to_first_frame"], 200.0);
		let metrics = summary["metrics"].as_array().unwrap();
		let tick = metrics
			.iter()
			.find(|metric| metric["name"] == "GraphicsApplication::tick")
			.unwrap();
		assert_eq!((tick["kind"].as_str(), tick["count"].as_u64()), (Some("cpu"), Some(2)));
		assert_eq!(
			(tick["min"].as_f64(), tick["max"].as_f64(), tick["mean"].as_f64()),
			(Some(2.0), Some(3.0), Some(2.5))
		);
		let bloom = metrics.iter().find(|metric| metric["name"] == "pass.Bloom").unwrap();
		assert_eq!(
			(bloom["kind"].as_str(), bloom["count"].as_u64(), bloom["p99"].as_f64()),
			(Some("gpu"), Some(1), Some(0.5))
		);

		let presented = server.get_json("/metrics?presented=true");
		assert_eq!(presented["ticks"], 1);
		assert_eq!(presented["first_tick"], 1);

		let response = server.request("GET", "/metrics?window=5", "");
		assert!(response.starts_with(b"HTTP/1.1 400"), "unexpected response: {response:?}");
	}

	#[test]
	fn server_streams_metric_rows_as_ndjson() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		record_test_ticks(inspector.metrics());
		let server = TestServer::new(inspector);

		let response = server.request("GET", "/metrics/frames", "");
		let (headers, body) = split_response(&response);
		assert!(headers.starts_with("HTTP/1.1 200"), "unexpected response: {headers}");
		assert!(
			headers.to_ascii_lowercase().contains("content-type: application/x-ndjson"),
			"unexpected headers: {headers}"
		);
		let rows = std::str::from_utf8(body)
			.unwrap()
			.lines()
			.map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
			.collect::<Vec<_>>();
		assert_eq!(rows.len(), 2);
		assert_eq!(rows[0]["tick"], 0);
		assert_eq!(rows[0]["frame"], 0);
		assert_eq!(rows[0]["cpu"]["GraphicsApplication::tick"], 2.0);
		assert_eq!(rows[0]["gpu"]["pass.Bloom"], 0.5);
		assert_eq!(rows[1]["presented"], true);
		assert!(rows[1]["gpu"].as_object().unwrap().is_empty());

		let response = server.request("GET", "/metrics/frames?since=0&limit=1", "");
		let (_, body) = split_response(&response);
		assert_eq!(std::str::from_utf8(body).unwrap().lines().count(), 1);
		let response = server.request("GET", "/metrics/frames?limit=1", "");
		let (_, body) = split_response(&response);
		let row: serde_json::Value = serde_json::from_str(std::str::from_utf8(body).unwrap().trim()).unwrap();
		assert_eq!(row["tick"], 1);
	}

	/// Ends tick `tick`, which submitted frame `tick`, the way the graphics loop does.
	fn end_test_tick(inspector: &DefaultInspector, tick: u64) {
		inspector.metrics().end_tick(TickClock {
			tick,
			frame: Some(tick),
			time: Duration::from_millis(100 * (tick + 1)),
			delta: Duration::from_micros(16_667),
			presented: true,
		});
		inspector.event_signal().notify();
	}

	/// Returns the `event` name of each event in an events response.
	fn event_names(batch: &serde_json::Value) -> Vec<&str> {
		batch["events"]
			.as_array()
			.unwrap()
			.iter()
			.map(|event| event["event"].as_str().unwrap())
			.collect()
	}

	#[test]
	fn events_report_ticks_completed_frames_and_configuration_changes_after_a_cursor() {
		let configuration = Configuration::new();
		let port = configuration.register("render.pass.");
		let id = configuration.update("render.pass.bloom", "bypassed");
		let (inspector, _events, _transforms) = test_inspector(configuration);
		record_test_ticks(inspector.metrics());
		inspector.metrics().complete_frame(0);
		let server = TestServer::new(inspector.clone());

		let start = server.get_json("/events");
		assert_eq!(event_names(&start), ["configuration"]);
		assert_eq!(start["events"][0]["data"]["parameter"], "render.pass.bloom");
		assert_eq!(start["events"][0]["data"]["state"]["status"], "pending");

		let bloom = inspector.metrics().register(MetricKind::Gpu, "pass.Bloom").unwrap();
		inspector.metrics().set_gpu(1, bloom, Duration::from_micros(250));
		inspector.metrics().complete_frame(1);
		end_test_tick(&inspector, 2);
		port.set(id, "bypassed".into());

		let batch = server.get_json(&format!("/events?after={}", start["cursor"].as_str().unwrap()));
		assert_eq!(event_names(&batch), ["gpu", "tick", "configuration"]);
		assert_eq!(batch["events"][0]["data"]["tick"], 1);
		assert_eq!(batch["events"][0]["data"]["gpu"]["pass.Bloom"], 0.25);
		assert_eq!(batch["events"][1]["data"]["tick"], 2);
		assert_eq!(batch["events"][2]["data"]["state"]["status"], "set");

		let filtered = server.get_json(&format!("/events?after={}&events=tick", start["cursor"].as_str().unwrap()));
		assert_eq!(event_names(&filtered), ["tick"]);

		let empty = server.get_json(&format!("/events?after={}&wait=20", batch["cursor"].as_str().unwrap()));
		assert_eq!(event_names(&empty), [] as [&str; 0]);
		assert_eq!(empty["cursor"], batch["cursor"]);
	}

	#[test]
	fn events_wait_until_a_tick_ends() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let server = TestServer::new(inspector.clone());
		let cursor = server.get_json("/events")["cursor"].as_str().unwrap().to_owned();

		let ticker = std::thread::spawn({
			let inspector = inspector.clone();
			move || {
				std::thread::sleep(Duration::from_millis(100));
				end_test_tick(&inspector, 0);
			}
		});
		let started = std::time::Instant::now();
		let batch = server.get_json(&format!("/events?after={cursor}&wait=900"));
		ticker.join().unwrap();

		assert_eq!(event_names(&batch), ["tick"]);
		assert!(
			started.elapsed() < Duration::from_millis(800),
			"the tick did not end the wait"
		);
	}

	#[test]
	fn events_report_lagged_ticks_and_closing() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let server = TestServer::new(inspector.clone());
		let cursor = server.get_json("/events")["cursor"].as_str().unwrap().to_owned();
		let ticks = crate::metrics::TICK_CAPACITY as u64 + 6;
		for tick in 0..ticks {
			end_test_tick(&inspector, tick);
		}

		let batch = server.get_json(&format!("/events?after={cursor}&events=configuration"));
		assert_eq!(event_names(&batch), ["lagged"]);
		assert_eq!(batch["events"][0]["data"]["first"], 6);
		assert_eq!(batch["events"][0]["data"]["missed"], 6);

		inspector.event_signal().close();
		let started = std::time::Instant::now();
		let closing = server.get_json(&format!("/events?after={}&wait=900", batch["cursor"].as_str().unwrap()));
		assert_eq!(event_names(&closing), ["closing"]);
		assert!(closing["events"][0].get("data").is_none(), "closing carries no data");
		assert!(started.elapsed() < Duration::from_millis(800), "closing did not end the wait");
	}

	#[test]
	fn events_refuse_waiters_beyond_the_limit_and_malformed_queries() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let server = Arc::new(TestServer::new(inspector.clone()));
		let cursor = server.get_json("/events")["cursor"].as_str().unwrap().to_owned();

		let waiters = (0..crate::inspector::events::MAX_EVENT_WAITERS)
			.map(|_| {
				let (server, cursor) = (Arc::clone(&server), cursor.clone());
				std::thread::spawn(move || server.get_json(&format!("/events?after={cursor}&wait=600")))
			})
			.collect::<Vec<_>>();
		std::thread::sleep(Duration::from_millis(200));
		let response = server.request("GET", &format!("/events?after={cursor}&wait=600"), "");
		assert!(response.starts_with(b"HTTP/1.1 503"), "unexpected response: {response:?}");
		for waiter in waiters {
			assert_eq!(event_names(&waiter.join().unwrap()), [] as [&str; 0]);
		}

		for query in ["events=bogus", "after=1-2", "after=a-b-c", "wait=30001", "since=0"] {
			let response = server.request("GET", &format!("/events?{query}"), "");
			assert!(
				response.starts_with(b"HTTP/1.1 400"),
				"{query}: unexpected response: {response:?}"
			);
		}
	}

	#[test]
	fn server_rejects_zero_entity_targets() {
		let (inspector, _events, mut transforms) = test_inspector(Configuration::new());
		let server = TestServer::new(inspector);
		let body = format!(
			r#"{{"type":"{TRANSFORMATION_UPDATE_MESSAGE_TYPE}","target":0,"payload":{{"position":[4.0,5.0,6.0],"scale":[1.0,1.0,1.0],"orientation":[0.0,0.0,0.0,1.0]}}}}"#
		);
		let response = server.request("POST", "/messages", &body);
		assert!(response.starts_with(b"HTTP/1.1 400"));
		assert!(transforms.read().is_none());
	}

	#[test]
	fn server_posts_targeted_transform_updates_over_http() {
		let (inspector, _events, mut transforms) = test_inspector(Configuration::new());
		let server = TestServer::new(inspector);
		let target = Handle::from_id(47);
		let body = format!(
			r#"{{"type":"{TRANSFORMATION_UPDATE_MESSAGE_TYPE}","target":{},"payload":{{"position":[4.0,5.0,6.0],"scale":[1.0,2.0,3.0],"orientation":[0.0,0.0,0.0,1.0]}}}}"#,
			target.id()
		);
		let response = server.request("POST", "/messages", &body);
		assert!(response.starts_with(b"HTTP/1.1 204"), "unexpected response: {response:?}");
		let update = transforms.read().expect("posted transform update");
		assert_eq!(update.handle(), target);
		assert_eq!(update.transform().get_position(), math::Point::new(4.0, 5.0, 6.0));
		assert_eq!(update.transform().scale(), math::Scale::new(1.0, 2.0, 3.0));
	}

	#[test]
	fn server_posts_reflected_destroy_messages_and_retires_the_entity() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("http-destroy-test-world");
		let deletion_messages = messages.channel::<DeleteMessage>();
		let mut deletions = deletion_messages.listener();
		let entities = messages.factory::<String>();
		let mut inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			messages,
			Arc::new(Metrics::new()),
		);
		inspector
			.register_message(DESTROY_MESSAGE_TYPE, deletion_messages)
			.expect("register reflected destroy message");
		let inspector = EntityHandle::from(inspector);
		let server = TestServer::new(inspector.clone());
		let target = entities.create("temporary".to_string());
		let body = format!(
			r#"{{"type":"{DESTROY_MESSAGE_TYPE}","target":{},"payload":null}}"#,
			target.id()
		);
		let response = server.request("POST", "/messages", &body);
		assert!(response.starts_with(b"HTTP/1.1 204"), "unexpected response: {response:?}");
		assert_eq!(deletions.read().expect("posted deletion").into_handle(), target);
		assert!(inspector.entities(None, None).is_empty());
	}

	/// Builds the capture request a query is expected to parse into.
	fn capture(sink: usize, capture: ScreenshotCapture, format: ScreenshotFormat) -> super::CaptureRequest {
		super::CaptureRequest {
			selection: ScreenshotSelection { sink, capture },
			format,
		}
	}

	#[test]
	fn screenshot_query_decodes_fields_in_any_order() {
		for (query, pass, target) in [
			("sink=2&pass=bloom&target=main", "bloom", "main"),
			("target=main&sink=2&pass=bloom", "bloom", "main"),
			("sink=2&pass=atmosphere%20sky&target=lit%20main", "atmosphere sky", "lit main"),
			("sink=2&pass=atmosphere+sky&target=lit+main", "atmosphere sky", "lit main"),
		] {
			assert_eq!(
				super::parse_screenshot_query(Some(query)),
				Ok(capture(
					2,
					ScreenshotCapture::AfterPass {
						pass: pass.to_string(),
						target: target.to_string(),
					},
					ScreenshotFormat::Png,
				))
			);
		}
	}

	#[test]
	fn entity_query_parses_valid_filters_and_rejects_malformed_ones() {
		assert_eq!(
			super::parse_entity_query(Some("class=alloc%3A%3Astring%3A%3AString&name=shipping+crate")),
			Ok(super::EntityQuery {
				entity_type: Some("alloc::string::String".to_string()),
				name: Some("shipping crate".to_string()),
			})
		);
		for query in [
			"type=String&class=String",
			"name=crate&name=barrel",
			"unknown=String",
			"type=",
			"type=bad%2G",
		] {
			assert_eq!(super::parse_entity_query(Some(query)), Err(()));
		}
	}

	#[test]
	fn screenshot_query_rejects_incomplete_and_malformed_fields() {
		assert_eq!(super::parse_screenshot_query(None), Err(()));
		for query in [
			"sink=2&pass=bad%&target=main",
			"sink=2&pass=bad%2G&target=main",
			"sink=2&pass=%FF&target=main",
			"sink=2&pass=bloom",
			"sink=2&pass=bloom&target=main&extra=x",
			"sink=2&sink=3",
			"sink=2&previous=true",
			"sink=2&pass=bloom&target=main&previous=true",
			"sink=2&target=main&previous=yes",
			"sink=2&format=tiff",
		] {
			assert_eq!(super::parse_screenshot_query(Some(query)), Err(()));
		}
	}

	#[test]
	fn server_lists_debug_views() {
		let catalog = Arc::new(crate::rendering::DebugViewCatalog::default());
		let (inspector, _events, _transforms) = test_inspector_with(Configuration::new(), Some(Arc::clone(&catalog)));
		let server = TestServer::new(inspector);

		let body = server.get_json("/debug-views");
		assert_eq!(body["enabled"], false);
		assert_eq!(body["views"].as_array().map(Vec::len), Some(0));
	}

	#[test]
	fn server_exposes_configuration_event_values() {
		let configuration = Configuration::new();
		let _port = configuration.register("render.pass.");
		configuration.update("render.pass.bloom", "bypassed");
		let (inspector, _events, _transforms) = test_inspector(configuration);
		let body = TestServer::new(inspector).get_json("/configuration");
		assert_eq!(body[0]["parameter"], "render.pass.bloom");
		assert_eq!(body[0]["requested"], "bypassed");
		assert_eq!(body[0]["state"]["status"], "pending");
	}

	#[test]
	fn posted_configuration_reaches_the_system_that_owns_its_prefix() {
		let configuration = Configuration::new();
		let port = configuration.register("render.lod.");
		let (inspector, _events, _transforms) = test_inspector(configuration);
		let server = TestServer::new(inspector);

		let response = server.request("POST", "/configuration", r#"{"parameter":"render.lod.force","value":0}"#);
		assert!(response.starts_with(b"HTTP/1.1 202"), "unexpected response: {response:?}");
		let event: serde_json::Value = serde_json::from_slice(response_body(&response)).expect("parse configuration event");
		assert_eq!(event["parameter"], "render.lod.force");
		assert_eq!(event["state"]["status"], "pending");

		let update = port.read().expect("the owning port receives the posted update");
		assert_eq!(update.parameter(), "force");
		assert_eq!(update.value(), &crate::configuration::ConfigurationValue::Integer(0));
		port.set(update.id(), update.value().clone());
		let events = server.get_json("/configuration");
		assert_eq!(events[0]["state"], serde_json::json!({ "status": "set", "value": 0 }));
	}

	#[test]
	fn posted_configuration_without_an_owner_is_not_set() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let response = TestServer::new(inspector).request(
			"POST",
			"/configuration",
			r#"{"parameter":"render.unknown.knob","value":"on"}"#,
		);
		assert!(response.starts_with(b"HTTP/1.1 202"), "unexpected response: {response:?}");
		let event: serde_json::Value = serde_json::from_slice(response_body(&response)).expect("parse configuration event");
		assert_eq!(event["state"]["status"], "not-set");
	}

	#[test]
	fn configuration_request_rejects_unknown_fields() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let response = TestServer::new(inspector).request(
			"POST",
			"/configuration",
			r#"{"parameter":"render.lod.force","value":0,"when":"now"}"#,
		);
		assert!(response.starts_with(b"HTTP/1.1 400"), "unexpected response: {response:?}");
	}
}
