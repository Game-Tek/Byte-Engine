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
	core::EntityHandle,
	inspector::{
		Inspector,
		screenshot::{ScreenshotCapture, ScreenshotError, ScreenshotSubmitError},
	},
};

const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// The `HttpInspectorServer` struct exposes the Byte Engine Inspection Protocol
/// through an HTTP API.
///
/// Clients use this server to inspect registered entities and update their
/// exposed properties.
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

		let mut server = Server::new(move |request| match (request.method(), request.uri().path()) {
			(&Method::GET, "/screenshots") => screenshot_response(&i, request.uri().query()),
			(&Method::GET, "/configuration") => match serde_json::to_string(&i.configuration_events()) {
				Ok(body) => Response::builder().body(Body::from(body)).unwrap(),
				Err(error) => Response::builder()
					.status(StatusCode::INTERNAL_SERVER_ERROR)
					.body(Body::from(format!(
						"Configuration events could not be serialized. The most likely cause is an unsupported configuration value: {error}"
					)))
					.unwrap(),
			},
			(&Method::GET, "/entities") => {
				let mut body = String::new();

				let class_name = if let Some(pq) = request.uri().path_and_query() {
					if let Some(query) = pq.query() {
						let mut split = query.split("=");

						let filter = split.next().unwrap_or("");
						let value = split.next().unwrap_or("");

						if filter.starts_with("class") { Some(value) } else { None }
					} else {
						None
					}
				} else {
					None
				};

				let entities = i.get_entities(class_name);

				if !entities.is_empty() {
					for (index, entity) in entities.iter().enumerate() {
						body.push_str(&format!("[{}] {}\n", index, entity.as_string()));
					}
				} else {
					body.push_str("No entities found");
				}

				Response::builder().body(Body::from(body)).unwrap()
			}
			(&Method::PATCH, "/entities") => {
				if let Some(pq) = request.uri().path_and_query() {
					if let Some(query) = pq.query() {
						let mut params = query.split('&');

						let mut index_qp = params.next().unwrap().split('=');
						let _ = index_qp.next().unwrap();
						let index = index_qp.next().unwrap();

						let mut key_qp = params.next().unwrap().split('=');
						let _ = key_qp.next().unwrap();
						let key = key_qp.next().unwrap();

						let mut value_qp = params.next().unwrap().split('=');
						let _ = value_qp.next().unwrap();
						let value = value_qp.next().unwrap();

						match i.call_set(index.parse().unwrap_or(0), key, value) {
							Ok(_) => Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap(),
							Err(e) => Response::builder()
								.status(StatusCode::INTERNAL_SERVER_ERROR)
								.body(Body::from(e))
								.unwrap(),
						}
					} else {
						Response::builder()
							.status(StatusCode::BAD_REQUEST)
							.body(Body::empty())
							.unwrap()
					}
				} else {
					Response::builder()
						.status(StatusCode::BAD_REQUEST)
						.body(Body::empty())
						.unwrap()
				}
			}
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
			channel::DefaultChannel,
			listener::{DefaultListener, Listener as _},
		},
		inspector::{Inspector, screenshot::Screenshot},
	};

	/// Creates an inspector with a live future-only application event listener.
	fn test_inspector(configuration: Configuration) -> (EntityHandle<Inspector>, DefaultListener<Events>) {
		let events = DefaultChannel::new();
		let listener = events.listener();
		(EntityHandle::from(Inspector::new(events, configuration)), listener)
	}

	#[test]
	fn server_answers_entity_requests_over_http() {
		// Reserve an available local port so the test exercises the real socket path without competing for the production port.
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let (inspector, _events) = test_inspector(Configuration::new());
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
		assert!(response.ends_with("No entities found"), "unexpected response: {response}");
	}

	#[test]
	fn server_publishes_application_close_requests() {
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let (inspector, mut events) = test_inspector(Configuration::new());
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

		let (inspector, _events) = test_inspector(Configuration::new());
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
		let (inspector, _events) = test_inspector(Configuration::new());
		let response = super::screenshot_response(&inspector, None);
		assert_eq!(response.status(), oxhttp::model::StatusCode::BAD_REQUEST);
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
		let (inspector, _events) = test_inspector(configuration);
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
		assert!(response.contains("\"parameter\":\"render.pass.bloom\""));
		assert!(response.contains("\"requested\":\"bypassed\""));
		assert!(response.contains("\"status\":\"pending\""));
	}
}
