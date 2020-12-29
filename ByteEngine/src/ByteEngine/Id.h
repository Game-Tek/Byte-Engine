#pragma once

#include "Core.h"
#include <GTSL/Id.h>
#include <GTSL/ShortString.hpp>

class Id
{
public:
	Id() = default;
	
	template<uint64 N>
	constexpr Id(char const (&s)[N]) : hashedName(s) {}
	
	constexpr Id(const UTF8* name) noexcept : hashedName(name), stringName(name) {}
	Id(const GTSL::Id64 name) noexcept : hashedName(name) {}
	explicit Id(const uint64 value) noexcept : hashedName(value) {}

	[[nodiscard]] const UTF8* GetString() const { return GTSL::Range<const UTF8*>(stringName).begin(); }
	[[nodiscard]] GTSL::Id64 GetHash() const { return hashedName; }

	operator GTSL::Id64() const { return hashedName; }
	explicit operator const UTF8* () const { return GTSL::Range<const UTF8*>(stringName).begin(); }

	operator uint64() const { return hashedName; }

	Id& operator=(const UTF8* name) { hashedName = name; stringName = name; return *this; }
	Id& operator=(const GTSL::Id64 other) { hashedName = other; return *this; }
	
	bool operator==(const Id other) const { return hashedName == other.hashedName; }
	bool operator==(const GTSL::Id64 other) const { return hashedName == other; }
private:
	GTSL::Id64 hashedName;
	GTSL::ShortString<24> stringName;
};
