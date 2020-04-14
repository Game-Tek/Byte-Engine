#pragma once

#include "NetworkCore.h"

class NetSocket
{
	uint64 handle = 0;
public:
	struct CreateInfo
	{
		uint16 Port = 0;
		bool Blocking = false;
	};
	
	NetSocket(const CreateInfo& createInfo);
	~NetSocket();

	struct SendInfo
	{
		IpEndpoint Endpoint;
		void* Data = nullptr;
		uint32 Size = 0;
	};
	bool Send(const SendInfo& sendInfo) const;


	struct ReceiveInfo
	{
		IpEndpoint* Sender = nullptr;
		void* Buffer = nullptr;
		uint32 BufferSize = 0;
	};
	bool Receive(const ReceiveInfo& receiveInfo) const;
};
