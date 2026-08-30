use std::{
	io,
	net::{Ipv4Addr, Ipv6Addr, SocketAddr},
	time::Duration,
};

use oxhttp::{
	ListeningServer, Server,
	model::{Body, Method, Response, StatusCode},
};

use crate::{
	core::{EntityHandle, factory::Handle},
	inspector::{
		Inspector,
		screenshot::{ScreenshotCapture, ScreenshotError, ScreenshotSubmitError},
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
/// since the previous request. Payloads remain opaque.
///
/// `POST /messages` accepts message types registered through
/// [`Inspector::register_message`]. To move an entity, send a JSON object with
/// `type: "TransformationUpdate"`, its numeric `target` handle, and the complete
/// reflected [`Transform`](crate::gameplay::Transform) payload.
/// See the [HTTP Inspector API](/docs/api/inspector) for every endpoint and payload.
pub struct HttpInspectorServer {
	_server: ListeningServer,

	_inspector: EntityHandle<Inspector>,
}

impl HttpInspectorServer {
	/// Starts the inspector on the loopback interface at port 6680.
	///
	/// Next, request `GET /entities` to verify that the application is available.
	pub fn new(inspector: EntityHandle<Inspector>) -> Self {
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
	fn spawn(inspector: EntityHandle<Inspector>, addresses: impl IntoIterator<Item = SocketAddr>) -> io::Result<Self> {
		let i = inspector.clone();

		let mut server = Server::new(move |mut request| match (request.method(), request.uri().path()) {
			(&Method::GET, "/screenshots") => screenshot_response(&i, request.uri().query()),
			(&Method::GET, "/messages") => messages_response(&i),
			(&Method::POST, "/messages") => message_response(&i, request.body_mut()),
			(&Method::GET, "/configuration") => match serde_json::to_string(&i.configuration_events()) {
				Ok(body) => Response::builder()
					.header("Content-Type", "application/json")
					.body(Body::from(body))
					.unwrap(),
				Err(error) => Response::builder()
					.status(StatusCode::INTERNAL_SERVER_ERROR)
					.body(Body::from(format!(
						"Configuration events could not be serialized. The most likely cause is an unsupported configuration value: {error}"
					)))
					.unwrap(),
			},
			(&Method::GET, "/entities") => entities_response(&i, request.uri().query()),
			(&Method::DELETE, "/") => {
				i.close_application();
				Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap()
			}
			_ => Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap(),
		});

		for address in addresses {
			server = server.bind(address);
		}
		server = server.with_global_timeout(Duration::from_secs(10));
		server = server.with_max_concurrent_connections(8);

		let server = server.spawn()?;

		Ok(Self {
			_server: server,
			_inspector: inspector,
		})
	}
}

/// Serializes the current factory-backed entity catalog.
fn entities_response(inspector: &Inspector, query: Option<&str>) -> Response<Body> {
	let query = match parse_entity_query(query) {
		Ok(query) => query,
		Err(()) => {
			return response(
				StatusCode::BAD_REQUEST,
				"Entity query is malformed. The most likely cause is an unknown, duplicate, empty, or invalidly encoded `type`, `class`, or `name` parameter.",
			);
		}
	};
	let entities = inspector
		.entities(query.entity_type.as_deref(), query.name.as_deref())
		.iter()
		.map(|entity| {
			serde_json::json!({
				"target": entity.handle().id(),
				"name": entity.name(),
				"types": entity.types(),
			})
		})
		.collect::<Vec<_>>();
	json_response(&serde_json::Value::Array(entities))
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

	let mut parsed = EntityQuery::default();
	for parameter in query.split('&') {
		let (name, value) = parameter.split_once('=').ok_or(())?;
		let value = decode_query_component(value)?;
		if value.is_empty() {
			return Err(());
		}
		let slot = match name {
			"type" | "class" => &mut parsed.entity_type,
			"name" => &mut parsed.name,
			_ => return Err(()),
		};
		if slot.replace(value).is_some() {
			return Err(());
		}
	}
	Ok(parsed)
}

/// Drains and serializes passive message publications without payloads.
fn messages_response(inspector: &Inspector) -> Response<Body> {
	let batch = inspector.drain_messages();
	let messages = batch
		.messages()
		.iter()
		.map(|message| {
			serde_json::json!({
				"topic": message.topic_id(),
				"scope": message.scope(),
				"type": message.message_type(),
				"first_sequence": message.first_sequence(),
				"count": message.count(),
			})
		})
		.collect::<Vec<_>>();
	json_response(&serde_json::json!({ "messages": messages }))
}

fn json_response(value: &serde_json::Value) -> Response<Body> {
	Response::builder()
		.status(StatusCode::OK)
		.header("Content-Type", "application/json")
		.body(Body::from(value.to_string()))
		.expect("Inspector JSON response is valid. The most likely cause of failure is an invalid static header name.")
}

/// Parses and posts one complete message envelope from the HTTP request body.
fn message_response(inspector: &Inspector, body: &mut Body) -> Response<Body> {
	let request: serde_json::Value = match serde_json::from_reader(body) {
		Ok(request) => request,
		Err(error) => {
			return response(
				StatusCode::BAD_REQUEST,
				&format!(
					"Inspector message request is invalid JSON. The most likely cause is a malformed request body: {error}"
				),
			);
		}
	};
	let (message_type, target, payload) = match parse_message_request(&request) {
		Ok(request) => request,
		Err(error) => return response(StatusCode::BAD_REQUEST, error),
	};

	match inspector.post_message(message_type, target, payload) {
		Ok(()) => Response::builder()
			.status(StatusCode::NO_CONTENT)
			.body(Body::empty())
			.unwrap(),
		Err(error) => response(StatusCode::BAD_REQUEST, &error),
	}
}

/// Validates the protocol envelope before a typed payload parser sees it.
fn parse_message_request(request: &serde_json::Value) -> Result<(&str, Handle, &serde_json::Value), &'static str> {
	let object = request.as_object().ok_or(INVALID_MESSAGE_REQUEST)?;
	if object.len() != 3
		|| object
			.keys()
			.any(|key| !matches!(key.as_str(), "type" | "target" | "payload"))
	{
		return Err(INVALID_MESSAGE_REQUEST);
	}
	let message_type = object
		.get("type")
		.and_then(serde_json::Value::as_str)
		.filter(|value| !value.is_empty())
		.ok_or(INVALID_MESSAGE_REQUEST)?;
	let target = object
		.get("target")
		.and_then(serde_json::Value::as_u64)
		.and_then(|target| u32::try_from(target).ok())
		.map(Handle::from_id)
		.ok_or(INVALID_MESSAGE_REQUEST)?;
	let payload = object.get("payload").ok_or(INVALID_MESSAGE_REQUEST)?;
	Ok((message_type, target, payload))
}

const INVALID_MESSAGE_REQUEST: &str = "Inspector message request is invalid. The most likely cause is that it must contain only a non-empty string `type`, an unsigned 32-bit `target`, and a `payload`.";

/// Handles one screenshot request after HTTP routing has selected the endpoint.
fn screenshot_response(inspector: &Inspector, query: Option<&str>) -> Response<Body> {
	let (sink, capture) = match parse_screenshot_query(query) {
		Ok(request) => request,
		Err(()) => {
			return response(
				StatusCode::BAD_REQUEST,
				"Screenshot query is malformed. The most likely cause is a missing sink, an unknown or duplicate parameter, or specifying only one of `pass` and `target`.",
			);
		}
	};
	let response_receiver = match inspector.screenshots().request(sink, capture) {
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
		Ok(Err(ScreenshotError::Internal(error))) => response(StatusCode::INTERNAL_SERVER_ERROR, &error),
		Err(_) => response(
			StatusCode::GATEWAY_TIMEOUT,
			"Screenshot request timed out. The most likely cause is that the graphics thread did not complete a frame before the deadline.",
		),
	}
}

/// Parses the complete screenshot query without accepting unknown or duplicate parameters.
fn parse_screenshot_query(query: Option<&str>) -> Result<(usize, ScreenshotCapture), ()> {
	let mut sink = None;
	let mut pass = None;
	let mut target = None;
	for parameter in query.ok_or(())?.split('&') {
		let (name, value) = parameter.split_once('=').ok_or(())?;
		let value = decode_query_component(value)?;
		if value.is_empty() {
			return Err(());
		}
		let slot = match name {
			"sink" => &mut sink,
			"pass" => &mut pass,
			"target" => &mut target,
			_ => return Err(()),
		};
		if slot.replace(value).is_some() {
			return Err(());
		}
	}
	let sink = sink.ok_or(())?.parse().map_err(|_| ())?;
	let capture = match (pass, target) {
		(None, None) => ScreenshotCapture::FinalSwapchain,
		(Some(pass), Some(target)) => ScreenshotCapture::AfterPass { pass, target },
		_ => return Err(()),
	};
	Ok((sink, capture))
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

fn response(status: StatusCode, message: &str) -> Response<Body> {
	Response::builder()
		.status(status)
		.body(Body::from(message.to_string()))
		.expect("Inspector HTTP error response is valid. The most likely cause of failure is an invalid status code.")
}

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		net::{Ipv4Addr, TcpListener, TcpStream},
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
			message_bus::MessageBus,
		},
		gameplay::{Name, TransformationUpdate},
		inspector::{Inspector, TRANSFORMATION_UPDATE_MESSAGE_TYPE, screenshot::Screenshot},
	};

	/// Creates an inspector with live future-only control and transform listeners.
	fn test_inspector(
		configuration: Configuration,
	) -> (
		EntityHandle<Inspector>,
		DefaultListener<Events>,
		DefaultListener<TransformationUpdate>,
	) {
		let events = DefaultChannel::new();
		let event_listener = events.listener();
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("http-inspector-test-world");
		let transform_listener = messages.channel().listener();
		let mut inspector = Inspector::new(events, configuration, messages);
		inspector
			.register_message::<TransformationUpdate>(TRANSFORMATION_UPDATE_MESSAGE_TYPE)
			.expect("register reflected transformation update");
		(EntityHandle::from(inspector), event_listener, transform_listener)
	}

	#[test]
	fn server_answers_entity_requests_over_http() {
		// Reserve an available local port so the test exercises the real socket path without competing for the production port.
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let _server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");

		let mut stream = TcpStream::connect(address).expect("connect to inspector test server");
		stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		stream
			.write_all(b"GET /entities HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
			.expect("request inspector entities");

		let mut response = String::new();
		stream.read_to_string(&mut response).expect("read inspector response");

		assert!(response.starts_with("HTTP/1.1 200"), "unexpected response: {response}");
		assert!(response.ends_with("[]"), "unexpected response: {response}");
	}

	#[test]
	fn server_reports_factory_entities_and_generic_message_publications() {
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("http-observation-test");
		let entity = messages.factory::<String>().create("crate".to_string());
		let generic_messages = messages.channel::<Option<u32>>();
		let _generic_listener = generic_messages.listener();
		generic_messages.send(Some(7));
		let inspector = EntityHandle::from(Inspector::new(DefaultChannel::new(), Configuration::new(), messages));
		let _server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");

		let mut entities_stream = TcpStream::connect(address).expect("connect to inspector entity endpoint");
		entities_stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		entities_stream
			.write_all(
				b"GET /entities?type=alloc%3A%3Astring%3A%3AString HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
			)
			.expect("request observed entities");
		let mut entities_response = String::new();
		entities_stream
			.read_to_string(&mut entities_response)
			.expect("read observed entities");
		let entities_body = entities_response.split_once("\r\n\r\n").expect("entity response body").1;
		let entities: serde_json::Value = serde_json::from_str(entities_body).expect("parse observed entities");
		assert_eq!(entities[0]["target"], entity.id());
		assert_eq!(entities[0]["types"][0], std::any::type_name::<String>());

		let mut messages_stream = TcpStream::connect(address).expect("connect to inspector message endpoint");
		messages_stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		messages_stream
			.write_all(b"GET /messages HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
			.expect("request observed messages");
		let mut messages_response = String::new();
		messages_stream
			.read_to_string(&mut messages_response)
			.expect("read observed messages");
		let messages_body = messages_response.split_once("\r\n\r\n").expect("message response body").1;
		let messages: serde_json::Value = serde_json::from_str(messages_body).expect("parse observed messages");
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
		let inspector = Inspector::new(DefaultChannel::new(), Configuration::new(), messages);

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
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let (inspector, mut events, _transforms) = test_inspector(Configuration::new());
		let _server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");
		let mut stream = TcpStream::connect(address).expect("connect to inspector test server");
		stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		stream
			.write_all(b"DELETE / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
			.expect("request application close");

		let mut response = String::new();
		stream.read_to_string(&mut response).expect("read inspector response");

		assert!(response.starts_with("HTTP/1.1 200"), "unexpected response: {response}");
		assert_eq!(events.read(), Some(Events::Close));
	}

	#[test]
	fn server_returns_screenshot_with_capture_headers() {
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let screenshots = inspector.screenshots();
		let _server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");
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

		let mut stream = TcpStream::connect(address).expect("connect to inspector test server");
		stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		stream
			.write_all(b"GET /screenshots?sink=2 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
			.expect("request screenshot");
		let mut response = Vec::new();
		stream.read_to_end(&mut response).expect("read screenshot response");
		responder.join().expect("join screenshot responder");

		let headers_end = response
			.windows(4)
			.position(|window| window == b"\r\n\r\n")
			.expect("response headers")
			+ 4;
		let headers = std::str::from_utf8(&response[..headers_end]).expect("UTF-8 headers");
		assert!(headers.starts_with("HTTP/1.1 200"), "unexpected response: {headers}");
		assert!(headers.contains("content-type: image/png"));
		assert!(headers.contains("x-byte-engine-frame: 41"));
		assert!(headers.contains("x-byte-engine-sink: 2"));
		assert_eq!(&response[headers_end..], b"fake-png");
	}

	#[test]
	fn server_rejects_missing_screenshot_sink() {
		let (inspector, _events, _transforms) = test_inspector(Configuration::new());
		let response = super::screenshot_response(&inspector, None);
		assert_eq!(response.status(), oxhttp::model::StatusCode::BAD_REQUEST);
	}

	#[test]
	fn server_posts_targeted_transform_updates_over_http() {
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let (inspector, _events, mut transforms) = test_inspector(Configuration::new());
		let _server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");
		let target = Handle::from_id(47);
		let body = format!(
			r#"{{"type":"{TRANSFORMATION_UPDATE_MESSAGE_TYPE}","target":{},"payload":{{"position":[4.0,5.0,6.0],"scale":[1.0,2.0,3.0],"orientation":[0.0,0.0,0.0,1.0]}}}}"#,
			target.id()
		);
		let request = format!(
			"POST /messages HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
			body.len()
		);

		let mut stream = TcpStream::connect(address).expect("connect to inspector test server");
		stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		stream.write_all(request.as_bytes()).expect("post inspector transform update");
		let mut response = String::new();
		stream.read_to_string(&mut response).expect("read inspector response");

		assert!(response.starts_with("HTTP/1.1 204"), "unexpected response: {response}");
		let update = transforms.read().expect("posted transform update");
		assert_eq!(update.handle(), target);
		assert_eq!(update.transform().get_position(), math::Point::new(4.0, 5.0, 6.0));
		assert_eq!(update.transform().scale(), math::Scale::new(1.0, 2.0, 3.0));
	}

	#[test]
	fn screenshot_query_accepts_the_pair_in_any_order() {
		use crate::inspector::screenshot::ScreenshotCapture;

		for query in ["sink=2&pass=bloom&target=main", "target=main&sink=2&pass=bloom"] {
			assert_eq!(
				super::parse_screenshot_query(Some(query)),
				Ok((
					2,
					ScreenshotCapture::AfterPass {
						pass: "bloom".to_string(),
						target: "main".to_string(),
					}
				))
			);
		}
	}

	#[test]
	fn entity_query_rejects_duplicate_and_malformed_filters() {
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
	fn entity_query_combines_type_and_name_filters() {
		assert_eq!(
			super::parse_entity_query(Some("class=alloc%3A%3Astring%3A%3AString&name=shipping+crate")),
			Ok(super::EntityQuery {
				entity_type: Some("alloc::string::String".to_string()),
				name: Some("shipping crate".to_string()),
			})
		);
	}

	#[test]
	fn screenshot_query_decodes_form_components() {
		use crate::inspector::screenshot::ScreenshotCapture;

		for query in [
			"sink=2&pass=atmosphere%20sky&target=lit%20main",
			"sink=2&pass=atmosphere+sky&target=lit+main",
		] {
			assert_eq!(
				super::parse_screenshot_query(Some(query)),
				Ok((
					2,
					ScreenshotCapture::AfterPass {
						pass: "atmosphere sky".to_string(),
						target: "lit main".to_string(),
					}
				))
			);
		}
	}

	#[test]
	fn screenshot_query_rejects_malformed_escapes_and_invalid_utf8() {
		for query in [
			"sink=2&pass=bad%&target=main",
			"sink=2&pass=bad%2G&target=main",
			"sink=2&pass=%FF&target=main",
		] {
			assert_eq!(super::parse_screenshot_query(Some(query)), Err(()));
		}
	}

	#[test]
	fn screenshot_query_requires_both_pass_and_target_and_rejects_extras() {
		for query in [
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
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let configuration = Configuration::new();
		let _port = configuration.register("render.pass.");
		configuration.update("render.pass.bloom", "bypassed");
		let (inspector, _events, _transforms) = test_inspector(configuration);
		let _server = HttpInspectorServer::spawn(inspector, [address]).expect("start inspector test server");

		let mut stream = TcpStream::connect(address).expect("connect to inspector test server");
		stream
			.set_read_timeout(Some(Duration::from_secs(1)))
			.expect("set inspector response timeout");
		stream
			.write_all(b"GET /configuration HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
			.expect("request inspector configuration");

		let mut response = String::new();
		stream.read_to_string(&mut response).expect("read inspector response");

		assert!(response.starts_with("HTTP/1.1 200"), "unexpected response: {response}");
		assert!(response.contains("content-type: application/json"));
		assert!(response.contains("\"parameter\":\"render.pass.bloom\""));
		assert!(response.contains("\"requested\":\"bypassed\""));
		assert!(response.contains("\"status\":\"pending\""));
	}
}
