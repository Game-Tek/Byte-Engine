use std::{net::{Ipv4Addr, Ipv6Addr}, sync::Arc, time::Duration};

use oxhttp::{model::{Body, Method, Response, StatusCode}, ListeningServer, Server};
use utils::sync::Mutex;

use crate::{application::Events, camera::Camera, core::{listener::{CreateEvent, Listener}, Entity, EntityHandle}, inspector::Inspectable};

pub struct HttpInspectorServer {
	server: ListeningServer,

	entities: Arc<Mutex<Vec<EntityHandle<dyn Inspectable>>>>,
}

impl HttpInspectorServer {
    pub fn new(tx: std::sync::mpmc::Sender<Events>) -> Self {
    	let entities = Arc::new(Mutex::new(Vec::<EntityHandle<dyn Inspectable>>::with_capacity(32768)));

     	let l = entities.clone();

     	let mut server = Server::new(move |request| {
			match (request.method(), request.uri().path()) {
				(&Method::GET, "/entities") => {
					let entities = l.lock();
					let mut body = String::new();

					let pe = if let Some(pq) = request.uri().path_and_query() {
						if let Some(query) = pq.query() {
							let mut split = query.split("=");

							let filter = split.next().unwrap_or("");
							let value = split.next().unwrap_or("");

							if filter.starts_with("class") {
								entities.iter().filter(|e| e.read().class_name() == value).collect::<Vec<_>>()
							} else {
								entities.iter().collect::<Vec<_>>()
							}
						} else {
							entities.iter().collect::<Vec<_>>()
						}
					} else {
						entities.iter().collect::<Vec<_>>()
					};

					if !pe.is_empty() {
						for entity in pe {
							body.push_str(&format!("{}\n", entity.read().as_string()));
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

							let entities = l.lock();

							if let Some(e) = entities.get(index.parse().unwrap_or(0)) {
								match e.write().set(key, value) {
									Ok(_) => Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap(),
									Err(e) => Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::from(e)).unwrap()
								}
							} else {
								Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap()
							}
						} else {
							Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap()
						}
					} else {
						Response::builder().status(StatusCode::BAD_REQUEST).body(Body::empty()).unwrap()
					}
				}
				(&Method::POST, "/close") => {
					tx.send(Events::Close).unwrap();
					Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap()
				}
				_ => Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap()
			}
		});

		server = server.bind((Ipv4Addr::LOCALHOST, 8080)).bind((Ipv6Addr::LOCALHOST, 8080));
		server = server.with_global_timeout(Duration::from_secs(10));
		server = server.with_max_concurrent_connections(8);

		let server = server.spawn().unwrap();

        Self {
            server,

            entities,
        }
    }
}

impl Entity for HttpInspectorServer {
	fn builder(self) -> crate::core::entity::EntityBuilder<'static, Self> where Self: Sized {
    	crate::core::entity::EntityBuilder::new(self).listen_to::<CreateEvent<dyn Inspectable>>()
	}
}

impl Listener<CreateEvent<dyn Inspectable>> for HttpInspectorServer {
	fn handle(&mut self, event: &CreateEvent<dyn Inspectable>) {
		self.entities.lock().push(event.handle().clone());
	}
}
