use std::{net::{Ipv4Addr, Ipv6Addr}, sync::Arc, time::Duration};

use oxhttp::{model::{Body, Method, Response, StatusCode}, ListeningServer, Server};
use utils::sync::Mutex;

use crate::{application::Events, camera::Camera, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, inspector::{Inspectable, Inspector}};

/// This is the HTTP based implementation of the Byte Engine Inspection Protocol.
///
/// This implementation provides a RESTful API for inspecting and managing entities within the Byte Engine application.
/// It allows clients to retrieve information about entities and update existing entities.
pub struct HttpInspectorServer {
	server: ListeningServer,

	inspector: EntityHandle<Inspector>,
}

impl HttpInspectorServer {
	pub fn new(inspector: EntityHandle<Inspector>) -> Self {
		let i = inspector.clone();

		let mut server = Server::new(move |request| {
			match (request.method(), request.uri().path()) {
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

					let entities = i.read().get_entities(class_name);

					if !entities.is_empty() {
						for (index, entity) in entities.iter().enumerate() {
							body.push_str(&format!("[{}] {}\n", index, entity.read().as_string()));
						}
					} else {
						body.push_str("No entities found");
					}

					Response::builder().body(Body::from(body)).unwrap()
				},
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

							match i.read().call_set(index.parse().unwrap_or(0), key, value) {
								Ok(_) => Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap(),
								Err(e) => Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::from(e)).unwrap()
							}
						} else {
							Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap()
						}
					} else {
						Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap()
					}
				}
				(&Method::POST, "/close") => {
					i.read().close_application();
					Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap()
				}
				_ => Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap()
			}
		});

		server = server.bind((Ipv4Addr::LOCALHOST, 6680)).bind((Ipv6Addr::LOCALHOST, 6680));
		server = server.with_global_timeout(Duration::from_secs(10));
		server = server.with_max_concurrent_connections(8);

		let server = server.spawn().unwrap();

		Self {
			server,

			inspector,
		}
	}
}

impl Entity for HttpInspectorServer {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self)
	}
}
