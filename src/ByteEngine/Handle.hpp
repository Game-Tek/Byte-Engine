#pragma once

#include <concepts>
#include "Core.h"

template<typename C, typename TAG>
class Handle
{
public:
	Handle() = default;
	~Handle() = default;

	explicit Handle(C value) noexcept : m_handle(value) {}

	explicit operator C() const { return m_handle; }

	C operator()() const { return m_handle; }

	bool operator==(const Handle& other) const { return m_handle == other.m_handle; }
	bool operator!=(const Handle& other) const { return m_handle != other.m_handle; }
private:
	C m_handle;

	friend Handle;
};

template<std::integral C, typename TAG>
class Handle<C, TAG>
{
public:
	Handle() = default;
	~Handle() = default;

	explicit Handle(C value) noexcept : m_handle(value) {}

	explicit operator C() const { return m_handle; }

	C operator()() const { return m_handle; }

	bool operator==(const Handle& other) const { return m_handle == other.m_handle; }
	bool operator!=(const Handle& other) const { return m_handle != other.m_handle; }

	explicit operator bool() const { return m_handle != static_cast<C>(~0); }
private:
	C m_handle = static_cast<C>(~0);
};

#define MAKE_HANDLE(type, name) using name##Handle = Handle<type, struct name##_tag>;