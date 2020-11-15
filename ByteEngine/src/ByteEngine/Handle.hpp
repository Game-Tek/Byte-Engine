#pragma once

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

#define MAKE_HANDLE(type, name)\
	struct name##_tag{};\
	using name##Handle = Handle<type, name##_tag>;