#pragma once

#include "Core.h"

namespace GTSL
{
	class String;

	class Id64
	{
	public:
		using HashType = uint64;

		constexpr Id64() = default;
		constexpr Id64(const char* cstring) noexcept;
		constexpr explicit Id64(HashType id) noexcept;
		explicit Id64(const String& string);
		constexpr Id64(const Id64& other) = default;
		constexpr Id64(Id64&& other) noexcept : hashValue(other.hashValue) { other.hashValue = 0; }

		~Id64() noexcept = default;

		constexpr Id64& operator=(const Id64& other) noexcept = default;
		constexpr bool operator==(const Id64& other) const noexcept { return hashValue == other.hashValue; }
		constexpr Id64& operator=(Id64&& other) noexcept { hashValue = other.hashValue; other.hashValue = 0; return *this; }

		constexpr HashType GetID() noexcept { return hashValue; }
		[[nodiscard]] constexpr HashType GetID() const noexcept { return hashValue; }

		constexpr operator HashType() const { return hashValue; }

		constexpr static HashType HashString(const char* text) noexcept;
		static HashType HashString(const String& string) noexcept;

	private:
		HashType hashValue = 0;

		constexpr static HashType hashString(uint32 length, const char* text) noexcept;
	};

	class Id32
	{
		uint32 hash = 0;
		constexpr static uint32 hashString(uint32 stringLength, const char* str) noexcept;
	public:
		constexpr Id32(const char* text) noexcept;
		constexpr Id32(uint32 length, const char* text) noexcept;

		constexpr operator uint32() const noexcept { return hash; }
	};

	class Id16
	{
		uint16 hash = 0;
		static uint16 hashString(uint32 stringLength, const char* str);
	public:
		Id16(const char* text);

		operator uint16() const { return hash; }
	};
}