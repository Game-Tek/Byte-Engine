#pragma once

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

template<typename TAG>
class Handle<uint32, TAG>
{
public:
	Handle() = default;
	~Handle() = default;

	explicit Handle(uint32 value) noexcept : handle(value) {}

	explicit operator uint32() const { return handle; }

	uint32 operator()() const { return handle; }

	bool operator==(const Handle & other) const { return handle == other.handle; }
	bool operator!=(const Handle & other) const { return handle != other.handle; }

	explicit operator bool() const { return handle != 0xFFFFFFFF; }
private:
	uint32 handle = 0xFFFFFFFF;
};

#define MAKE_HANDLE(type, name)\
	using name##Handle = Handle<type, struct name##_tag>;