#pragma once

#include "Core.h"

#include <WinSock2.h>

class NetSocket
{
	
};

struct IPv4
{
	uint8 nums[8];

	uint8 operator[](const uint8 index) { return nums[index]; }
};

struct IPv6
{
	uint8 nums[16];

	uint8 operator[](const uint8 index) { return nums[index]; }
};