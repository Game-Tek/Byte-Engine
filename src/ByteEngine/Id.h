#pragma once

#include "Core.h"
#include <GTSL/Id.h>
#include <GTSL/ShortString.hpp>

class Id
{
public:
	Id() = default;
	
	template<uint64 N>
	constexpr explicit Id(char8_t const (&s)[N]) : hashedName(s) {}
	
	constexpr explicit Id(const utf8* name) noexcept : hashedName(name), stringName(name) {}
	constexpr explicit Id(const GTSL::Range<const utf8*> name) noexcept : hashedName(name), stringName(name) {}
	Id(const GTSL::Id64 name) noexcept : hashedName(name) {}
	//explicit Id(const uint64 value) noexcept : hashedName(value) {}

	//[[nodiscard]] const utf8* GetString() const { return GTSL::Range<const utf8*>(stringName).begin(); }
	[[nodiscard]] constexpr GTSL::Id64 GetHash() const { return hashedName; }
	
	explicit operator GTSL::Id64() const { return hashedName; }
	//explicit operator const utf8* () const { return GTSL::Range<const utf8*>(stringName).(); }
	explicit operator GTSL::Range<const utf8*>() const { return stringName; }
	explicit operator bool() const { return hashedName.GetID(); }

	Id& operator=(const utf8* name) { hashedName = name; stringName = GTSL::Range(name); return *this; }
	Id& operator=(const GTSL::Id64 other) { hashedName = other; return *this; }
	
	bool operator==(const Id other) const { return hashedName == other.hashedName; }
	//bool operator==(const GTSL::Id64 other) const { return hashedName == other; }

	uint64 operator()() const { return hashedName.GetID(); }
	explicit operator uint64() const { return hashedName(); }
private:
	GTSL::Id64 hashedName;
	GTSL::ShortString<64> stringName;
};

namespace GTSL {
	template<>
	struct Hash<Id> {
		uint64 value = 0;
		constexpr Hash(const Id& id) : value(id.GetHash()) {}
		constexpr operator uint64() const { return value; }
	};

	Hash(Id) -> Hash<Id>;
}