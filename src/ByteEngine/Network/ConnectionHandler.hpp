#include "ByteEngine/Game/System.hpp"
#include <GTSL/Network/Sockets.h>

class ConnectionHandler : public BE::System {
public:
	ConnectionHandler(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"ConnectionHandler"), clients(16, GetPersistentAllocator())
	{
		source.Address[0] = 127;
		source.Address[1] = 0;
		source.Address[2] = 0;
		source.Address[3] = 1;
		source.Port = 25565;

		socket.Open(source, false);
	}

	void poll()
	{
		GTSL::IPv4Endpoint sender;

		socket.Receive(&sender, {});
	}

	enum class ConnectionAttemptCodes {
		OK, NO_MORE_SLOTS, ALREADY_EXISTS
	};

	MAKE_HANDLE(uint32, Connection);

	GTSL::Result<ConnectionHandle, ConnectionAttemptCodes> OpenConnection(GTSL::StringView connection_name, const GTSL::IPv4Endpoint endpoint) {
		if (clients.GetLength() == maxClients) { return { ConnectionHandle(), ConnectionAttemptCodes::NO_MORE_SLOTS }; } //if we already exhausted our client capacity, fail connection

		auto adressLookup = lookupClientBasedOnAddress(endpoint);
		if (adressLookup) { return { ConnectionHandle(), ConnectionAttemptCodes::ALREADY_EXISTS }; } //if connection with address exists, reject

		//todo: check salt

		auto clientIndex = clients.GetLength();

		auto& client = clients.EmplaceBack(GetPersistentAllocator());
		
		client.Name = connection_name;
		client.Salt = GTSL::Math::Random();
		client.ConnectionState = ClientData::ConnectionStates::CONNECTING;

		return { ConnectionHandle(clientIndex), ConnectionAttemptCodes::OK };
	}

private:
	static constexpr uint32 BUFFER_CAPACITY = 1024u, ACK_DEPTH = 32u;

	struct Header {
		/**
		 * \brief Increments with each packet sent, used to check ordering. Wraps around when overflowed.
		 */
		uint16 Sequence;

		/**
		 * \brief Stores the most recent packet sequence number received.
		 */
		uint16 LastSequenceNumberReceived;

		/**
		 * \brief Signals whether each one of the last 32 consecutive packets where received.
		 */
		GTSL::Bitfield<ACK_DEPTH> AckBits;
	};

	GTSL::Socket socket;

	GTSL::IPv4Endpoint source;

	//server
	uint32 maxClients = 0;

#undef NULL

	struct ClientData {
		ClientData(const BE::PAR& allocator) : Name(allocator) {}

		GTSL::String<BE::PAR> Name;
		uint64 Salt = 0;
		GTSL::IPv4Endpoint Address;
		enum class ConnectionStates { NULL, CONNECTING, OK, LOST } ConnectionState;
	};
	Vector<ClientData> clients;

	/**
	 * \brief Looks for a connected client matching the address provided.
	 * \param address Address to lookup
	 * \return Result<uint32>, number is client index, state is whether it was found
	 */
	GTSL::Result<uint32> lookupClientBasedOnAddress(const GTSL::IPv4Endpoint address) {
		if(auto r = GTSL::Find(clients, [address](const ClientData& client_data) { return client_data.Address.Address == address.Address && client_data.Address.Port == address.Port; })) {
			return { static_cast<uint32>(clients.end() - r.Get()), true };
		}

		return { 0, false };
	}
	//server

	//client
	uint32 sentSequenceBuffer[BUFFER_CAPACITY]{ ~0u };

	struct PacketData {
		bool Acknowledged = false;
		GTSL::Microseconds SendTime;
	};
	PacketData sentPacketBuffer[BUFFER_CAPACITY];

	PacketData* insertPacketData(uint16 sequence) {
		const  uint32 index = sequence % BUFFER_CAPACITY;
		sentSequenceBuffer[index] = sequence;
		return &sentPacketBuffer[index];
	}
	PacketData* getPacketData(const uint16 sequence) {
		const uint16 index = sequence % BUFFER_CAPACITY;

		if(sentSequenceBuffer[index] == sequence) {
			return &sentPacketBuffer[index];
		}

		return nullptr;
	}

	uint16 sendPacketSequenceNumber = 0;
	uint16 receivedPacketSequenceNumber = 0;
	//client

	static bool sequence_greater_than(uint16_t s1, uint16_t s2) {
		return ((s1 > s2) && (s1 - s2 <= 32768)) || ((s1 < s2) && (s2 - s1 > 32768));
	}

	//Unfortunately, on the receive side packets arrive out of order and some are lost.
	//Under ridiculously high packet loss (99%) I’ve seen old sequence buffer entries stick around from before the previous sequence number wrap at 65535 and break my ack logic
	//(leading to false acks and broken reliability where the sender thinks the other side has received something they haven’t…).
	//The solution to this problem is to walk between the previous highest insert sequence and the new insert sequence(if it is more recent)
	//and clear those entries in the sequence buffer to 0xFFFFFFFF.
	//Now in the common case, insert is very close to constant time, but worst case is linear where n is the number of sequence entries between the previous highest insert sequence and the current insert sequence.

	void processPacket() {
		Header header;

		if(sequence_greater_than(header.Sequence, receivedPacketSequenceNumber)) {
			//for(uint32 i = receivedPacketSequenceNumber; i < header.Sequence - 1; ++i) { //clear every slot in between
			//	sentSequenceBuffer[i] = ~0u;
			//}

			receivedPacketSequenceNumber = header.Sequence;

			sentSequenceBuffer[header.Sequence] = receivedPacketSequenceNumber;

			for(uint32 i = 0, j = header.LastSequenceNumberReceived; i < ACK_DEPTH; ++i, ++j) {
				bool val;

				header.AckBits.Get(ACK_DEPTH - 1 - i, val);

				if(val) {
					sentPacketBuffer[j % BUFFER_CAPACITY].Acknowledged = true;
				}
			}
		}
	}

	void sendPacket() {
		Header header;
		header.Sequence = sendPacketSequenceNumber;
		header.LastSequenceNumberReceived = receivedPacketSequenceNumber;

		for(uint32 i = 0; i < ACK_DEPTH; ++i) {
			auto& packet = sentPacketBuffer[(receivedPacketSequenceNumber - i) % BUFFER_CAPACITY];
			header.AckBits.Set(ACK_DEPTH - 1 - i, packet.Acknowledged);
		}

		++sendPacketSequenceNumber;
	}
};