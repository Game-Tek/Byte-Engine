//! This crate contains the implementation of the Byte Engine Transport Protocol (BETP) protocol.
//! The implementation is designed _sans-io_ and can be used with any I/O implementation.
//!
//! # Module Structure
//!
//! The module is divided into the following submodules:
//!
//! - `server`: Contains the implementation of the server.
//! - `client`: Contains the implementation of the client.
//! - `remote`: Contains the implementation of the remote connection.
//! - `local`: Contains the implementation of the local connection.
//! - `packets`: Contains the implementation of the packets used in the protocol.
//!
//! # Protocol Overview
//!
//! The Byte Engine Transport Protocol (BETP) is a simple, reliable, and ordered protocol that is used to transfer data between a client and a server.
//! The protocol is designed to be used in a client-server architecture where the server is the authoritative entity that manages connections to clients and maintains the state of the game.
//! The protocol is built on top of the User Datagram Protocol (UDP) and provides reliable and ordered delivery of packets.
//!
//! The protocol consists of the following packets in the following order:
//!
//! - Connection Request Packet: Sent by the client to request a connection to the server.
//! - Challenge Packet: Sent by the server to challenge the client.
//! - Challenge Response Packet: Sent by the client to respond to the challenge.
//! - Data Packet: Sent by the client or server to send data.
//! - Disconnect Packet: Sent by the client or server to update the connection status.
//!
//! The protocol uses sequence numbers to ensure that packets are delivered in order and to detect lost packets.
//! The protocol also uses acknowledgments to ensure that packets are reliably delivered.
//!
//! The protocol is designed to be simple and easy to implement, making it suitable for use in real-time multiplayer games.

#![feature(buf_read_has_data_left)]

pub mod client;
pub mod server;

mod local;
mod remote;

pub mod packets;

use std::{
    hash::{Hash, Hasher},
    io::{Read, Write},
    ops::Sub,
};

use self::packets::DataPacket;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// [`PacketInfo`] contains information about a packet.
/// - `acked`: A boolean that indicates if the packet has been acknowledged.
pub(crate) struct PacketInfo {
    acked: bool,
}

#[derive(Clone, Copy)]
enum ConnectionStates {
    Negotiating,
    Connected,
}

/// Compares two sequence numbers and returns true if the first sequence number is greater than the second.
/// The function takes into account the wrap-around of the sequence numbers.
pub(crate) fn sequence_greater_than(s1: u16, s2: u16) -> bool {
    ((s1 > s2) && (s1 - s2 <= 32768u16)) || ((s1 < s2) && (s2 - s1 > 32768u16))
}

fn has_written_anything(s: usize) -> Option<()> {
    if s > 0 {
        Some(())
    } else {
        None
    }
}

fn write_packet<const N: usize>(buffer: &mut [u8], packet_header: DataPacket<N>) -> Option<()> {
    let mut cursor = std::io::Cursor::new(buffer);

    {
        let protocol = &packet_header.header.protocol_id;
        let sequence = packet_header.connection_status.sequence.to_le_bytes();
        let ack = packet_header.connection_status.ack.to_le_bytes();
        let ack_bifield = packet_header.connection_status.ack_bitfield.to_le_bytes();

        cursor.write(protocol).ok().and_then(has_written_anything)?;
        cursor
            .write(&sequence)
            .ok()
            .and_then(has_written_anything)?;
        cursor.write(&ack).ok().and_then(has_written_anything)?;
        cursor
            .write(&ack_bifield)
            .ok()
            .and_then(has_written_anything)?;
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Read};

    use packets::{Packet, Packets};
    use tests::packets::{ConnectionStatus, PacketHeader, PacketType};

    use super::*;

    #[test]
    fn test_write_packet() {
        let mut buffer = [0u8; 12];
        write_packet(
            &mut buffer,
            DataPacket {
                header: PacketHeader {
                    protocol_id: [b'B', b'E', b'T', b'P'],
                    r#type: PacketType::Data,
                },
                connection_id: 0,
                connection_status: ConnectionStatus::new(0, 0, 0),
                data: [],
            },
        )
        .unwrap();

        let mut cursor = std::io::Cursor::new(&buffer);

        let mut protocol_id = [0u8; 4];
        assert_eq!(cursor.read(&mut protocol_id).unwrap(), 4);
        assert_eq!(&protocol_id, b"BETP");

        let mut sequence = [0u8; 2];
        assert_eq!(cursor.read(&mut sequence).unwrap(), 2);
        assert_eq!(u16::from_le_bytes([sequence[0], sequence[1]]), 0);

        let mut ack = [0u8; 2];
        assert_eq!(cursor.read(&mut ack).unwrap(), 2);
        assert_eq!(u16::from_le_bytes([ack[0], ack[1]]), 0);

        let mut ack_bitfield = [0u8; 4];
        assert_eq!(cursor.read(&mut ack_bitfield).unwrap(), 4);
        assert_eq!(
            u32::from_le_bytes([
                ack_bitfield[0],
                ack_bitfield[1],
                ack_bitfield[2],
                ack_bitfield[3]
            ]),
            0
        );

        assert!(!cursor.has_data_left().unwrap());
    }

    #[test]
    fn test_sequence_greater_than() {
        assert_eq!(sequence_greater_than(1, 0), true);
        assert_eq!(sequence_greater_than(0, 1), false);
        assert_eq!(sequence_greater_than(32768, 0), true);
        assert_eq!(sequence_greater_than(0, 32768), false);
        assert_eq!(sequence_greater_than(32767, 0), true);
        assert_eq!(sequence_greater_than(0, 32767), false);
        assert_eq!(sequence_greater_than(32767, 1), true);
        assert_eq!(sequence_greater_than(1, 32767), false);
    }

    #[test]
    fn test_server_client_link() {
		let client_address: std::net::SocketAddr = std::net::SocketAddr::from(([10, 0, 0, 1], 6669));
		let server_address: std::net::SocketAddr = std::net::SocketAddr::from(([210, 0, 0, 1], 6669));

    	let mut server = server::Server::new();
		let mut client = client::Client::new(server_address).unwrap();

		let request_packet = client.connect(std::time::Instant::now());

		assert_ne!(request_packet.get_client_salt(), 0);
		assert_eq!(request_packet.header().get_type(), PacketType::ConnectionRequest);

		let challenge_packet = server.handle_packet((client_address, Packets::ConnectionRequest(request_packet)), std::time::Instant::now()).unwrap().unwrap();

		assert!(matches!(challenge_packet, Packets::Challenge(_)));
		assert_eq!(challenge_packet.header().get_type(), PacketType::Challenge);

		let challenge_response_packet = client.handle_packet(challenge_packet).unwrap().unwrap();

		assert!(matches!(challenge_response_packet, Packets::ChallengeResponse(_)));
		assert_eq!(challenge_response_packet.header().get_type(), PacketType::ChallengeResponse);

		let data_packet = client.send([0; 1024]).unwrap();

		assert_eq!(data_packet.header().get_type(), PacketType::Data);
		assert_eq!(data_packet.get_connection_status().ack, 0);
		assert_eq!(data_packet.get_connection_status().ack_bitfield, 0b0);
		assert_eq!(data_packet.get_connection_status().sequence, 0);

		let response = server.handle_packet((client_address, Packets::Data(data_packet)), std::time::Instant::now()).unwrap();

		assert!(response.is_none());

		let data_packet = server.send(client_address, [0; 1024]).unwrap();

		assert_eq!(data_packet.header().get_type(), PacketType::Data);
		assert_eq!(data_packet.get_connection_status().ack, 0);
		assert_eq!(data_packet.get_connection_status().ack_bitfield, 0b1);
		assert_eq!(data_packet.get_connection_status().sequence, 0);

		let response = client.handle_packet(Packets::Data(data_packet)).unwrap();

		assert!(response.is_none());

		let data_packet = client.send([0; 1024]).unwrap();

		assert_eq!(data_packet.header().get_type(), PacketType::Data);
		assert_eq!(data_packet.get_connection_status().ack, 0);
		assert_eq!(data_packet.get_connection_status().ack_bitfield, 0b1);
		assert_eq!(data_packet.get_connection_status().sequence, 1);

		let response = server.handle_packet((client_address, Packets::Data(data_packet)), std::time::Instant::now()).unwrap();

		assert!(response.is_none());

		let data_packet = server.send(client_address, [0; 1024]).unwrap();

		assert_eq!(data_packet.header().get_type(), PacketType::Data);
		assert_eq!(data_packet.get_connection_status().ack, 1);
		assert_eq!(data_packet.get_connection_status().ack_bitfield, 0b11);
		assert_eq!(data_packet.get_connection_status().sequence, 1);

		let response = client.handle_packet(Packets::Data(data_packet)).unwrap();

		assert!(response.is_none());

		let disconnect = client.disconnect().unwrap();

		assert_eq!(disconnect.header().get_type(), PacketType::Disconnect);

		let response = server.handle_packet((client_address, Packets::Disconnect(disconnect)), std::time::Instant::now()).unwrap();

		assert!(response.is_none());
    }
}
