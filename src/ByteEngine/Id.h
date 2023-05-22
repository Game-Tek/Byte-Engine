#pragma once

#include "Core.h"
#include <GTSL/Id.h>
#include <GTSL/ShortString.hpp>
#include <GTSL/Range.hpp>
class Id
{
public:
	Id() = default;

	template<GTSL::uint64 N>
	constexpr explicit Id(char8_t const (&s)[N]) : m_hashedName(s) {}

	constexpr explicit Id(const char8_t* name) noexcept : m_hashedName(name),m_stringName(name) {}
	constexpr explicit Id(const GTSL::Range<const char8_t*> name) noexcept : m_hashedName(name), m_stringName(name) {}
	Id(const GTSL::Id64 name) noexcept : m_hashedName(name) {}

	[[nodiscard]] constexpr GTSL::Id64 GetHash() const { return m_hashedName; }

	explicit operator GTSL::Id64() const { return m_hashedName; }
	explicit operator GTSL::Range<const char8_t*>() const { return m_stringName; }
	explicit operator bool() const { return m_hashedName.GetID(); }

	Id& operator=(const char8_t* name) { m_hashedName = name; m_stringName = GTSL::Range(name); return *this; }
	Id& operator=(const GTSL::Id64 other) { m_hashedName = other; return *this; }

	bool operator==(const Id other) const { return m_hashedName == other.m_hashedName; }

	GTSL::uint64 operator()() const { return m_hashedName.GetID(); }
	explicit operator GTSL::uint64() const { return m_hashedName(); }
private:
	GTSL::Id64 m_hashedName;
	GTSL::ShortString<64> m_stringName;
};

namespace GTSL
{
	template<>
	struct Hash<Id>
	{
		uint64 value = 0;
		constexpr Hash(const Id& id) : value(id.GetHash()) {}
		constexpr operator uint64() const { return value; }
	};

	Hash(Id)->Hash<Id>;
}