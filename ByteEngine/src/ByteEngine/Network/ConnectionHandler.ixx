module;

#include "ByteEngine/Game/System.h"
#include <GTSL/Network/Sockets.h>

export module ConnectionHandler;

export class ConnectionHandler : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override
	{
		source.Address[0] = 127;
		source.Address[1] = 0;
		source.Address[2] = 0;
		source.Address[3] = 1;
		source.Port = 25565;

		GTSL::UDPSocket::CreateInfo createInfo;
		createInfo.Blocking = false;
		createInfo.Endpoint = source;
		socket.Open(createInfo);
	}

	void Shutdown(const ShutdownInfo& shutdownInfo)
	{
		socket.Close();
	}

	void poll()
	{
		GTSL::IPv4Endpoint sender;

		GTSL::UDPSocket::ReceiveInfo receiveInfo;
		receiveInfo.Buffer;
		receiveInfo.Sender = &sender;
		socket.Receive(receiveInfo);
	}

private:
	GTSL::UDPSocket socket;

	GTSL::IPv4Endpoint source;
};