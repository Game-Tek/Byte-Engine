use std::{
	io,
	net::{Ipv4Addr, Ipv6Addr, SocketAddr},
	time::Duration,
};

use oxhttp::{
	model::{Body, Method, Response, StatusCode},
	ListeningServer, Server,
};

use crate::{core::EntityHandle, inspector::Inspector};

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
			(&Method::GET, "/entities") => {
				let mut body = String::new();

				let class_name = if let Some(pq) = request.uri().path_and_query() {
					if let Some(query) = pq.query() {
						let mut split = query.split("=");

						let filter = split.next().unwrap_or("");
						let value = split.next().unwrap_or("");

						if filter.starts_with("class") {
							Some(value)
						} else {
							None
						}
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

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
		time::Duration,
	};

	use super::HttpInspectorServer;
	use crate::{application::Sender, core::EntityHandle, inspector::Inspector};

	#[test]
	fn server_answers_entity_requests_over_http() {
		// Reserve an available local port so the test exercises the real socket path without competing for the production port.
		let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve inspector test port");
		let address = reservation.local_addr().expect("read inspector test address");
		drop(reservation);

		let inspector = EntityHandle::from(Inspector::new(Sender::new(1)));
		let _server = HttpInspectorServer::spawn(inspector, [SocketAddr::from(address)]).expect("start inspector test server");

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
}
