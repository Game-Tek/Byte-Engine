#pragma once

#include "NetworkCore.h"


struct NetSocketCreateInfo
{
	uint16 Port = 0;
};

struct NetSocketSendInfo
{
	IpEndpoint Endpoint;
	void* Data = nullptr;
	uint32 Size = 0;
};

struct NetSocketReceiveInfo
{
	IpEndpoint* Sender = nullptr;
	void* Buffer = nullptr;
	uint32 BufferSize = 0;
};

class NetSocket
{
	uint64 handle = 0;
public:
	NetSocket(const NetSocketCreateInfo& NSCI_);
	~NetSocket();

	bool Send(const NetSocketSendInfo& NSSI_);
	bool Receive(const NetSocketReceiveInfo& NSRI_);
};
