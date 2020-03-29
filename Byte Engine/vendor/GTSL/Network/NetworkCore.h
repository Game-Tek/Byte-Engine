#pragma once

#include "Core.h"

/**
 * \brief Specifies and IP endpoint. This is an address and a port.
 * Used for socket connections.
 */
struct IpEndpoint
{
	uint8 Address[4]{};
	uint16 Port = 0;

	/**
	 * \brief Returns an element of the IP address from an index.
	 * \param index uint8 referring to which of the 4 bytes in the IP address to retrieve.
	 * \return uint8 containing the solicited element of the address.
	 */
	uint8 operator[](const uint8 index) { return Address[index]; }

	/**
	 * \brief Returns an int packed with the 4 byte values of the IP address this IpEndpoint holds.
	 * \return uint32 packed with the 4 byte values of the IP address this IpEndpoint holds.
	 */
	[[nodiscard]] uint32 IntFromAddress() const
	{
		return (Address[0] << 24) | (Address[1] << 16) | (Address[2] << 8) | Address[3];
	}

	/**
	 * \brief Sets this IpEndpoint's address as the passed in int_address_ uint32.
	 * \param int_address_ IP address packed in uint32 from which to build the address.
	 */
	void AddressFromInt(const uint32 int_address_)
	{
		Address[0] = ((int_address_ >> 24) & 0xFF), Address[1] = ((int_address_ >> 16) & 0xFF), Address[2] = ((
			int_address_ >> 8) & 0xFF), Address[3] = (int_address_ & 0xFF);
	}
};

struct IPv6
{
	uint8 nums[16];

	uint8 operator[](const uint8 index) { return nums[index]; }
};

/**
 * \brief Basic structure for a network packet in BE engine.
 */
struct Packet
{
	using ProtocolType = uint16;
	using SequenceType = uint16;
	using AckType = uint32;
	using BitFieldType = uint32;

	ProtocolType ProtocolID = 42069;
	SequenceType Sequence = 0;
	AckType Acknowledgment = 0;
	BitFieldType AckBitField = 0;
};
