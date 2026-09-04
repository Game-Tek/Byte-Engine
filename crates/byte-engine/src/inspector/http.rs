use std::{
	io,
	net::{Ipv4Addr, Ipv6Addr, SocketAddr},
	time::Duration,
};

use oxhttp::{
	ListeningServer, Server,
	model::{Body, Method, Response, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::{
	core::{EntityHandle, factory::Handle},
	inspector::{Inspector, ScreenshotCapture, ScreenshotError, ScreenshotSubmitError},
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
/// To drive an action created with
/// [`GraphicsApplication::create_action`](crate::application::graphics::GraphicsApplication::create_action),
/// resolve its target with `GET /entities?name=<action>` and send
/// `type: "TriggerAction"` with a reflected [`Value`](crate::input::Value).
///
/// The server retains only an [`Inspector`] trait object. Pass the same inspector
/// handle to another transport when clients need a second protocol surface.
/// See the [HTTP Inspector API](/docs/api/inspector) for every endpoint and payload.
pub struct HttpInspectorServer {
	_server: ListeningServer,
}

impl HttpInspectorServer {
	/// Starts the HTTP inspector transport on the loopback interface at port 6680.
	///
	/// Next, request `GET /entities` to verify that the application is available.
	pub fn new(inspector: EntityHandle<dyn Inspector>) -> Self {
		Self::spawn(
			inspector,
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
	fn spawn(inspector: EntityHandle<dyn Inspector>, addresses: impl IntoIterator<Item = SocketAddr>) -> io::Result<Self> {
		let mut server = Server::new(move |request| match (request.method(), request.uri().path()) {
			(&Method::GET, "/screenshots") => screenshot_response(&*inspector, request.uri().query()),
			(&Method::GET, "/messages") => messages_response(&*inspector),
			(&Method::GET, "/messages/types") => message_types_response(&*inspector),
			(&Method::POST, "/messages") => message_response(&*inspector, request.body_mut()),
			(&Method::GET, "/configuration") => json_response(&inspector.configuration_events()),
			(&Method::GET, "/entities") => entities_response(&*inspector, request.uri().query()),
			(&Method::DELETE, "/") => {
				inspector.close_application();
				response(StatusCode::OK, Body::empty())
			}
			_ => response(StatusCode::NOT_FOUND, Body::empty()),
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

/// Handles one screenshot request after HTTP routing has selected the endpoint.
fn screenshot_response(inspector: &dyn Inspector, query: Option<&str>) -> Response<Body> {
	let (sink, capture) = match parse_screenshot_query(query) {
		Ok(request) => request,
		Err(()) => {
			return response(
				StatusCode::BAD_REQUEST,
				"Screenshot query is malformed. The most likely cause is a missing sink, an unknown or duplicate parameter, or specifying only one of `pass` and `target`.",
			);
		}
	};
	let response_receiver = match inspector.request_screenshot(sink, capture) {
		Ok(receiver) => receiver,
		Err(ScreenshotSubmitError::QueueFull) => {
			return response(
				StatusCode::TOO_MANY_REQUESTS,
				"Screenshot queue is full. The most likely cause is that capture requests arrive faster than graphics frames can complete them.",
			);
		}
	};

	match response_receiver.recv_timeout(SCREENSHOT_TIMEOUT) {
		Ok(Ok(screenshot)) => Response::builder()
			.status(StatusCode::OK)
			.header("Content-Type", "image/png")
			.header("X-Byte-Engine-Frame", screenshot.frame.to_string())
			.header("X-Byte-Engine-Sink", sink.to_string())
			.body(Body::from(screenshot.png))
			.expect("Screenshot HTTP response is valid. The most likely cause of failure is an invalid static header name."),
		Ok(Err(ScreenshotError::SinkNotFound)) => response(
			StatusCode::NOT_FOUND,
			"Screenshot sink was not found. The most likely cause is that the sink index does not identify a renderer window.",
		),
		Ok(Err(ScreenshotError::SinkUnavailable)) => response(
			StatusCode::CONFLICT,
			"Screenshot sink is unavailable. The most likely cause is that its swapchain image could not be acquired for this frame.",
		),
		Ok(Err(ScreenshotError::PassNotFound)) => response(
			StatusCode::NOT_FOUND,
			"Screenshot render pass was not found. The most likely cause is that the selected sink has no pass with the requested name.",
		),
		Ok(Err(ScreenshotError::PassAmbiguous)) => response(
			StatusCode::CONFLICT,
			"Screenshot render pass is ambiguous. The most likely cause is that the selected sink has multiple passes with the requested name.",
		),
		Ok(Err(ScreenshotError::TargetNotWritten)) => response(
			StatusCode::NOT_FOUND,
			"Screenshot target was not written by the render pass. The most likely cause is that the target name is missing, read-only, or belongs to another pass.",
		),
		Ok(Err(ScreenshotError::Internal(error))) => response(StatusCode::INTERNAL_SERVER_ERROR, error),
		Err(_) => response(
			StatusCode::GATEWAY_TIMEOUT,
			"Screenshot request timed out. The most likely cause is that the graphics thread did not complete a frame before the deadline.",
		),
	}
}

/// Parses the complete screenshot query without accepting unknown or duplicate parameters.
fn parse_screenshot_query(query: Option<&str>) -> Result<(usize, ScreenshotCapture), ()> {
	let [sink, pass, target] = parse_query(query.ok_or(())?, [&["sink"], &["pass"], &["target"]])?;
	let sink = sink.ok_or(())?.parse().map_err(|_| ())?;
	match (pass, target) {
		(None, None) => Ok((sink, ScreenshotCapture::FinalSwapchain)),
		(Some(pass), Some(target)) => Ok((sink, ScreenshotCapture::AfterPass { pass, target })),
		_ => Err(()),
	}
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
		inspector::{DESTROY_MESSAGE_TYPE, DefaultInspector, Inspector, Screenshot, TRANSFORMATION_UPDATE_MESSAGE_TYPE},
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
			let server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");
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

	#[test]
	fn server_returns_screenshot_with_capture_headers() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let screenshots = inspector.screenshot_broker();
		let server = TestServer::new(inspector);
		let responder = std::thread::spawn(move || {
			let request = (0..100)
				.find_map(|_| {
					let request = screenshots.drain().pop();
					if request.is_none() {
						std::thread::sleep(Duration::from_millis(2));
					}
					request
				})
				.expect("receive screenshot request");
			request.complete(Ok(Screenshot {
				frame: 41,
				png: b"fake-png".to_vec(),
			}));
		});

		let response = server.request("GET", "/screenshots?sink=2", "");
		responder.join().expect("join screenshot responder");

		let headers_end = response.len() - response_body(&response).len();
		let headers = std::str::from_utf8(&response[..headers_end]).expect("UTF-8 headers");
		assert!(headers.starts_with("HTTP/1.1 200"), "unexpected response: {headers}");
		assert!(headers.contains("content-type: image/png"));
		assert!(headers.contains("x-byte-engine-frame: 41"));
		assert!(headers.contains("x-byte-engine-sink: 2"));
		assert_eq!(&response[headers_end..], b"fake-png");
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

	#[test]
	fn screenshot_query_decodes_fields_in_any_order() {
		use crate::inspector::screenshot::ScreenshotCapture;

		for (query, pass, target) in [
			("sink=2&pass=bloom&target=main", "bloom", "main"),
			("target=main&sink=2&pass=bloom", "bloom", "main"),
			("sink=2&pass=atmosphere%20sky&target=lit%20main", "atmosphere sky", "lit main"),
			("sink=2&pass=atmosphere+sky&target=lit+main", "atmosphere sky", "lit main"),
		] {
			assert_eq!(
				super::parse_screenshot_query(Some(query)),
				Ok((
					2,
					ScreenshotCapture::AfterPass {
						pass: pass.to_string(),
						target: target.to_string(),
					}
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
			"sink=2&target=main",
			"sink=2&pass=bloom&target=main&extra=x",
			"sink=2&sink=3",
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
