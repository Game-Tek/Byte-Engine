//! Loopback tests for the UDP [`Client`] and [`Server`] through real sockets.

use std::{
	net::{SocketAddr, UdpSocket},
	time::{Duration, Instant},
};

use betp::server::Events;

use super::{Client, Server, payload};

/// Runs enough client and server updates for loopback datagrams to finish a handshake or a payload round trip.
fn exchange(server: &mut Server, clients: &mut [&mut Client]) {
	for _ in 0..10 {
		for client in clients.iter_mut() {
			client
				.update(Instant::now())
				.expect("loopback client socket should stay usable");
		}
		server
			.update(Instant::now())
			.expect("loopback server socket should stay usable");
		std::thread::sleep(Duration::from_millis(1));
	}
}

fn loopback_server() -> (Server, SocketAddr) {
	let server = Server::bind("127.0.0.1:0").expect("a loopback port should be free");
	let address = server.local_addr().expect("a bound socket has an address");
	(server, address)
}

fn connected(server: &mut Server, address: SocketAddr) -> Client {
	let mut client = Client::connect(address).expect("a loopback client should bind");
	exchange(server, &mut [&mut client]);
	assert!(client.is_connected());
	client
}

#[test]
fn a_client_connects_and_exchanges_each_payload_once_in_both_directions() {
	let (mut server, address) = loopback_server();
	let mut client = connected(&mut server, address);
	let id = client.connection_id().expect("a connected client has an id");

	assert!(
		matches!(server.drain_events().collect::<Vec<_>>()[..], [Events::ClientConnected { id: connected }] if connected == id)
	);

	client.send(true, payload(7));
	server.send_to_client(id, true, payload(9));
	exchange(&mut server, &mut [&mut client]);
	// More updates retry any unacknowledged reliable send, which must not deliver it again.
	exchange(&mut server, &mut [&mut client]);

	assert_eq!(server.drain_received().collect::<Vec<_>>(), [(id, payload(7))]);
	assert_eq!(client.drain_received().collect::<Vec<_>>(), [payload(9)]);
}

#[test]
fn two_clients_share_one_server_socket_without_crossing_sessions() {
	let (mut server, address) = loopback_server();
	let mut client_a = connected(&mut server, address);
	let mut client_b = connected(&mut server, address);
	let (id_a, id_b) = (client_a.connection_id().unwrap(), client_b.connection_id().unwrap());

	assert_ne!(id_a, id_b);
	assert_eq!(server.connected_clients(), 2);

	client_a.send(false, payload(1));
	client_b.send(false, payload(2));
	server.send(false, payload(3));
	exchange(&mut server, &mut [&mut client_a, &mut client_b]);

	let mut received = server.drain_received().collect::<Vec<_>>();
	received.sort_by_key(|(_, data)| data[0]);
	assert_eq!(received, [(id_a, payload(1)), (id_b, payload(2))]);
	assert_eq!(client_a.drain_received().collect::<Vec<_>>(), [payload(3)]);
	assert_eq!(client_b.drain_received().collect::<Vec<_>>(), [payload(3)]);
}

#[test]
fn a_dropped_client_is_reported_disconnected_without_a_timeout() {
	let (mut server, address) = loopback_server();
	let client = connected(&mut server, address);
	let id = client.connection_id().unwrap();
	let _ = server.drain_events().count();

	drop(client);
	exchange(&mut server, &mut []);

	assert!(matches!(server.drain_events().collect::<Vec<_>>()[..], [Events::ClientDisconnected { id: gone }] if gone == id));
	assert_eq!(server.connected_clients(), 0);
}

#[test]
fn the_server_times_out_a_silent_client_and_tells_it_the_session_ended() {
	let (mut server, address) = loopback_server();
	let mut client = connected(&mut server, address);
	let id = client.connection_id().unwrap();
	let _ = server.drain_events().count();

	server.update(Instant::now() + Duration::from_secs(6)).unwrap();
	exchange(&mut server, &mut [&mut client]);

	assert!(matches!(server.drain_events().collect::<Vec<_>>()[..], [Events::ClientDisconnected { id: gone }] if gone == id));
	assert!(!client.is_connected());
}

#[test]
fn a_dropped_server_ends_its_clients_sessions() {
	let (mut server, address) = loopback_server();
	let mut client = connected(&mut server, address);

	drop(server);
	for _ in 0..10 {
		client.update(Instant::now()).unwrap();
		std::thread::sleep(Duration::from_millis(1));
	}

	assert!(!client.is_connected());
}

#[test]
fn a_client_started_before_the_server_connects_once_the_server_listens() {
	// Reserve a free port, then release it so the client first talks to a closed port.
	let address = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
	let mut client = Client::connect(address).unwrap();
	for _ in 0..3 {
		client.update(Instant::now()).unwrap();
		std::thread::sleep(Duration::from_millis(1));
	}

	let mut server = Server::bind(address).expect("the released port should still be free");
	exchange(&mut server, &mut [&mut client]);

	assert!(client.is_connected());
	assert_eq!(server.connected_clients(), 1);
}

#[test]
fn keepalives_hold_an_idle_session_open_past_the_timeout_without_reaching_the_application() {
	let (mut server, address) = loopback_server();
	let mut client = connected(&mut server, address);
	let _ = server.drain_events().count();
	let start = Instant::now();

	// Neither side has anything to send for longer than the timeout.
	for step in 1..=60 {
		let now = start + Duration::from_millis(100) * step;
		client.update(now).unwrap();
		server.update(now).unwrap();
		std::thread::sleep(Duration::from_millis(1));
	}

	assert!(client.is_connected());
	assert_eq!(server.connected_clients(), 1);
	assert_eq!(server.drain_events().count(), 0);
	assert_eq!(server.drain_received().count(), 0);
	assert_eq!(client.drain_received().count(), 0);
}

#[test]
fn a_client_ends_its_session_when_the_server_goes_quiet() {
	let (mut server, address) = loopback_server();
	let mut client = connected(&mut server, address);

	// The server stays bound but stops answering, as a frozen or crashed host would.
	client.update(Instant::now() + Duration::from_secs(6)).unwrap();

	assert!(!client.is_connected());
	assert_eq!(client.connection_id(), None);
}

#[test]
fn a_burst_of_reliable_payloads_larger_than_the_window_arrives_complete_and_in_order() {
	let (mut server, address) = loopback_server();
	let mut client = connected(&mut server, address);
	let count = betp::WINDOW * 3 + 5;

	for index in 0..count {
		client.send(true, payload(index as u8));
	}
	exchange(&mut server, &mut [&mut client]);

	let received = server.drain_received().map(|(_, data)| data[0]).collect::<Vec<_>>();
	assert_eq!(received, (0..count).map(|index| index as u8).collect::<Vec<_>>());
}
