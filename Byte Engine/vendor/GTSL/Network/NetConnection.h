#pragma once

#include "NetworkCore.h"
#include "Application/Clock.h"

class NetConnection
{
	Packet::SequenceType sequence = 0;

	float timeSince = 0.0f;
	float RTT = 0.0f;
	uint16 packetsLost = 0;

	static bool sequenceGreaterThan(const Packet::SequenceType a_, const Packet::SequenceType b_)
	{
		return ((a_ > b_) && (a_ - b_ <= 32768)) || ((a_ < b_) && (b_ - a_ > 32768));
	}

	static void setBit(Packet::AckType& a_, const uint8 bit_n_, const bool value_)
	{
		value_ ? a_ = a_ | (1 << bit_n_) : a_ = a_ & ~(1 << bit_n_);
	}

public:
	NetConnection();
	~NetConnection();

	[[nodiscard]] uint16 GetLostPacketCount() const { return packetsLost; }
	[[nodiscard]] float GetAverageRTT() const { return RTT; }
	[[nodiscard]] uint16 GetPing() const { return Clock::SecondsToMilliseconds(RTT / 2); }
};
