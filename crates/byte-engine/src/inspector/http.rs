use std::{
	io::{self, Write as _},
	net::{Ipv4Addr, Ipv6Addr, SocketAddr},
	time::Duration,
};

use oxhttp::{
	ListeningServer, Server,
	model::{Body, Method, Request, Response, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::{
	application::LoopWaker,
	core::{EntityHandle, factory::Handle},
	inspector::{
		Inspector, MAX_SCREENSHOT_CAPTURES, ScreenshotCapture, ScreenshotError, ScreenshotFormat, ScreenshotSelection,
		ScreenshotSubmitError,
	},
};

const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// The `HttpInspectorServer` struct exposes the Byte Engine Inspection Protocol
/// through an HTTP API.
///
/// Clients use this server to inspect factory-created entities and drain
/// passive publication ranges. `GET /entities` returns each numeric `target`
/// and its optional `name` plus Rust `types`. Filter entities with an exact
/// `name`, `type`, or both. `GET /messages` returns the `scope`, complete
/// generic `type`, `first_sequence`, and `count` for each route that published
/// since the previous request. Payloads remain opaque. `GET /messages/types`
/// returns each registered protocol `type` and the reflected shape of its JSON
/// `payload` so editor clients can build controls before posting a message.
///
/// `POST /messages` accepts message types registered through
/// [`Inspector::register_message`]. To move an entity, send a JSON object with
/// `type: "TransformationUpdate"`, its numeric `target` handle, and the complete
/// reflected [`Transform`](crate::gameplay::Transform) payload. To remove an
/// entity, send `type: "Delete"` or `type: "Destroy"` with its target and a
/// reflected unit payload represented by JSON `null`.
/// To drive a named action created through
/// [`GraphicsApplication::world`](crate::application::graphics::GraphicsApplication::world),
/// resolve its target with `GET /entities?name=<action>` and send `type:
/// "TriggerAction"` with a reflected [`Value`](crate::input::Value).
///
/// The server retains only an [`Inspector`] trait object. Pass the same inspector
/// handle to another transport when clients need a second protocol surface.
/// `GET /screenshots?sink=<index>` returns one image. Add `target=<name>` to read
/// a scene target, `pass=<name>` to read it right after that pass, and
/// `previous=true` to read the copy of a history target that the previous frame
/// wrote. Add `format=exr` for a lossless HDR image or `format=raw` for the GPU
/// bytes. To debug several targets from the same frame, `POST /screenshots` with
/// a JSON `captures` list; the response is `multipart/form-data` with one part
/// per capture.
///
/// See the [HTTP Inspector API](/docs/api/inspector) for every endpoint and payload.
pub struct HttpInspectorServer {
	_server: ListeningServer,
}

impl HttpInspectorServer {
	/// Starts the HTTP inspector transport on the loopback interface at port 6680.
	///
	/// Next, request `GET /entities` to verify that the application is available.
	pub fn new(inspector: EntityHandle<dyn Inspector>, waker: LoopWaker) -> Self {
		Self::spawn(
			inspector,
			waker,
			[
				SocketAddr::from((Ipv4Addr::LOCALHOST, 6680)),
				SocketAddr::from((Ipv6Addr::LOCALHOST, 6680)),
			],
		)
		.unwrap_or_else(|error| {
			panic!(
				"HTTP inspector could not start. The most likely cause is that port 6680 is already in use or unavailable: {error}"
			)
		})
	}

	/// Starts the inspector on each requested socket address.
	fn spawn(
		inspector: EntityHandle<dyn Inspector>,
		waker: LoopWaker,
		addresses: impl IntoIterator<Item = SocketAddr>,
	) -> io::Result<Self> {
		let mut server = Server::new(move |request| {
			let response = handle_request(&*inspector, &waker, request);
			// A request is input to the application, like a window event, so an idle loop runs after it.
			waker.wake();
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

/// Answers one inspector request.
fn handle_request(inspector: &dyn Inspector, waker: &LoopWaker, request: &mut Request<Body>) -> Response<Body> {
	match (request.method(), request.uri().path()) {
		(&Method::GET, "/screenshots") => screenshot_response(inspector, waker, request.uri().query()),
		(&Method::POST, "/screenshots") => screenshots_response(inspector, waker, request.body_mut()),
		(&Method::GET, "/messages") => messages_response(inspector),
		(&Method::GET, "/messages/types") => message_types_response(inspector),
		(&Method::POST, "/messages") => message_response(inspector, request.body_mut()),
		(&Method::GET, "/configuration") => json_response(&inspector.configuration_events()),
		(&Method::GET, "/entities") => entities_response(inspector, request.uri().query()),
		(&Method::DELETE, "/") => {
			inspector.close_application();
			response(StatusCode::OK, Body::empty())
		}
		_ => response(StatusCode::NOT_FOUND, Body::empty()),
	}
}

/// Serializes the current factory-backed entity catalog.
fn entities_response(inspector: &dyn Inspector, query: Option<&str>) -> Response<Body> {
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
fn messages_response(inspector: &dyn Inspector) -> Response<Body> {
	let messages = inspector.drain_messages();
	json_response(&serde_json::json!({ "messages": messages }))
}

/// Serializes the protocol message types accepted by the posting endpoint.
fn message_types_response(inspector: &dyn Inspector) -> Response<Body> {
	let types = inspector.message_types();
	json_response(&serde_json::json!({ "types": types }))
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
	payload: serde_json::Value,
}

/// Parses and posts one complete message envelope from the HTTP request body.
fn message_response(inspector: &dyn Inspector, body: &mut Body) -> Response<Body> {
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
	match inspector.post_message(&request.message_type, Handle::from_id(request.target), &request.payload) {
		Ok(()) => response(StatusCode::NO_CONTENT, Body::empty()),
		Err(error) => response(StatusCode::BAD_REQUEST, error),
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
fn screenshot_response(inspector: &dyn Inspector, waker: &LoopWaker, query: Option<&str>) -> Response<Body> {
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
fn screenshots_response(inspector: &dyn Inspector, waker: &LoopWaker, body: &mut Body) -> Response<Body> {
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
	inspector: &dyn Inspector,
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
		net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
		time::Duration,
	};

	use super::HttpInspectorServer;
	use crate::{
		application::Events,
		configuration::Configuration,
		core::{
			EntityHandle,
			channel::{Channel as _, DefaultChannel},
			factory::Handle,
			listener::{DefaultListener, Listener as _},
			message::DeleteMessage,
			message_bus::MessageBus,
		},
		gameplay::{Name, TransformationUpdate},
		inspector::{
			DESTROY_MESSAGE_TYPE, DefaultInspector, Inspector, ScreenshotCapture, ScreenshotError, ScreenshotFormat,
			ScreenshotSelection, Screenshots, TRANSFORMATION_UPDATE_MESSAGE_TYPE, screenshot::ScreenshotBroker,
		},
	};

	/// Creates an inspector with live future-only control and transform listeners.
	fn test_inspector(
		configuration: Configuration,
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
		let mut inspector = DefaultInspector::new(events, configuration, messages);
		inspector
			.register_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, transforms)
			.expect("register reflected transformation update");
		(EntityHandle::from(inspector), event_listener, transform_listener)
	}

	/// The `TestServer` struct keeps socket setup and raw HTTP exchange out of endpoint tests.
	struct TestServer {
		_server: HttpInspectorServer,
		address: SocketAddr,
	}

	impl TestServer {
		fn new(inspector: EntityHandle<dyn Inspector>) -> Self {
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
		let inspector = EntityHandle::from(DefaultInspector::new(DefaultChannel::new(), Configuration::new(), messages));
		let server = TestServer::new(inspector);
		let entities = server.get_json("/entities?type=alloc%3A%3Astring%3A%3AString");
		assert_eq!(entities[0]["target"], entity.id());
		assert_eq!(entities[0]["types"][0], std::any::type_name::<String>());

		let messages = server.get_json("/messages");
		assert_eq!(messages["messages"][0]["scope"], "http-observation-test");
		assert_eq!(messages["messages"][0]["type"], std::any::type_name::<Option<u32>>());
		assert_eq!(messages["messages"][0]["first_sequence"], 0);
		assert_eq!(messages["messages"][0]["count"], 1);
	}

	#[test]
	fn entity_endpoint_returns_and_filters_attached_names() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("named-http-entity-test");
		let labels = messages.factory::<String>();
		let names = messages.factory::<Name>();
		let inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), messages);

		let named = labels.create("crate-model".to_string());
		names.derive(named, Name::new("shipping crate"));
		let _unnamed = labels.create("barrel-model".to_string());

		let response = super::entities_response(&inspector, Some("name=shipping+crate"));
		assert_eq!(response.status(), oxhttp::model::StatusCode::OK);
		let entities: serde_json::Value = serde_json::from_reader(response.into_body()).expect("parse named entities");
		assert_eq!(entities.as_array().expect("entity array").len(), 1);
		assert_eq!(entities[0]["target"], named.id());
		assert_eq!(entities[0]["name"], "shipping crate");
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
			r#"{"captures":[{"sink":0,"target":"Contact Shadows","format":"exr"},{"sink":1,"pass":"bloom","target":"main","format":"raw"}]}"#,
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
		assert!(exr_headers.contains(r#"name="0"; filename="0-sink0-Contact_Shadows.exr""#));
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
			r#"{"captures":[{"sink":0},{"sink":0,"target":"Contact Shadows","previous":true}]}"#,
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
		let mut inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), messages);
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
	fn screenshot_query_without_a_pass_selects_a_scene_target() {
		assert_eq!(
			super::parse_screenshot_query(Some("sink=1&target=SSGI+History")),
			Ok(capture(
				1,
				ScreenshotCapture::SceneTarget {
					target: "SSGI History".to_string(),
				},
				ScreenshotFormat::Png,
			))
		);
		assert_eq!(super::parse_screenshot_query(Some("sink=1&pass=bloom")), Err(()));
	}

	#[test]
	fn screenshot_query_selects_a_previous_frame_target_and_an_encoding() {
		assert_eq!(
			super::parse_screenshot_query(Some("sink=0&target=Diffuse+Radiance+History&previous=true&format=exr")),
			Ok(capture(
				0,
				ScreenshotCapture::PreviousSceneTarget {
					target: "Diffuse Radiance History".to_string(),
				},
				ScreenshotFormat::Exr,
			))
		);
		assert_eq!(
			super::parse_screenshot_query(Some("sink=0&previous=false&format=raw")),
			Ok(capture(0, ScreenshotCapture::FinalSwapchain, ScreenshotFormat::Raw))
		);
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
}
