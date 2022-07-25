#pragma once

#include <concepts>

#include "ByteEngine/Core.h"

template<typename C, typename TAG>
class Handle
{
public:
	Handle() = default;
	~Handle() = default;

	explicit Handle(C value) noexcept : handle(value) {}
	
	explicit operator C() const { return handle; }

	C operator()() const { return handle; }
	
	bool operator==(const Handle& other) const { return handle == other.handle; }
	bool operator!=(const Handle& other) const { return handle != other.handle; }
private:
	C handle;

	friend Handle;
};

template<std::integral C, typename TAG>
class Handle<C, TAG>
{
public:
	Handle() = default;
	~Handle() = default;

	explicit Handle(C value) noexcept : handle(value) {}

	explicit operator C() const { return handle; }

	C operator()() const { return handle; }

	bool operator==(const Handle & other) const { return handle == other.handle; }
	bool operator!=(const Handle & other) const { return handle != other.handle; }

	explicit operator bool() const { return handle != static_cast<C>(~0); }
private:
	C handle = static_cast<C>(~0);
};

#define MAKE_HANDLE(type, name)\
	using name##Handle = Handle<type, struct name##_tag>;