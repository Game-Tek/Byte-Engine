#include "NetSocket.h"

#include <WinSock2.h>
#pragma comment(lib, "wsock32.lib")

NetSocket::NetSocket(const NetSocketCreateInfo& NSCI_)
{
	WSADATA WsaData;
	WSAStartup(MAKEWORD(2, 2), &WsaData);

	handle = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);

	sockaddr_in address{};
	address.sin_family = AF_INET;
	address.sin_addr.s_addr = INADDR_ANY;
	address.sin_port = htons(NSCI_.Port);

	if (bind(handle, reinterpret_cast<const sockaddr*>(&address), sizeof(sockaddr_in)) < 0)
	{
	}

	DWORD nonBlocking = 1;
	if (ioctlsocket(handle, FIONBIO, &nonBlocking) != 0)
	{
	}
}

NetSocket::~NetSocket()
{
	closesocket(handle);
}

bool NetSocket::Send(const NetSocketSendInfo& NSSI_)
{
	sockaddr_in addr;
	addr.sin_family = AF_INET;
	addr.sin_addr.s_addr = htonl(NSSI_.Endpoint.IntFromAddress());
	addr.sin_port = htons(NSSI_.Endpoint.Port);

	int sent_bytes = sendto(handle, static_cast<const char*>(NSSI_.Data), NSSI_.Size, 0,
	                        reinterpret_cast<sockaddr*>(&addr), sizeof(sockaddr_in));

	if (sent_bytes != NSSI_.Size) { return false; }

	return true;
}

bool NetSocket::Receive(const NetSocketReceiveInfo& NSRI_)
{
#if BE_PLATFORM_WIN
	typedef int socklen_t;
#endif

	sockaddr_in from;
	socklen_t fromLength = sizeof(from);

	int bytes = recvfrom(handle, reinterpret_cast<char*>(NSRI_.Buffer), NSRI_.BufferSize, 0,
	                     reinterpret_cast<sockaddr*>(&from), &fromLength);

	NSRI_.Sender->AddressFromInt(ntohl(from.sin_addr.s_addr));

	NSRI_.Sender->Port = ntohs(from.sin_port);

	return bytes;
}
