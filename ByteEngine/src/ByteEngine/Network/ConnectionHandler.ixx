module;

#include "ByteEngine/Game/System.h"
#include <GTSL/Network/Sockets.h>

export module ConnectionHandler;

export class ConnectionHandler : public System
{
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

	void Shutdown(const ShutdownInfo& shutdownInfo)
	{
	}

	void poll()
	{
		GTSL::IPv4Endpoint sender;

		socket.Receive(&sender, {});
	}

private:
	GTSL::UDPSocket socket;

	GTSL::IPv4Endpoint source;
};