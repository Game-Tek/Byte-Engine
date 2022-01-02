#include "ByteEngine/Game/System.hpp"
#include <GTSL/Network/Sockets.h>

class ConnectionHandler : public BE::System {
public:
	ConnectionHandler(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"ConnectionHandler")
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

private:
	GTSL::Socket socket;

	GTSL::IPv4Endpoint source;
};