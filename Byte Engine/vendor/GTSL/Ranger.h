#pragma once

template<typename T>
class Ranger
{
	const T* from = 0,* to = 0;
	
public:
	constexpr Ranger(T* start, T* end) noexcept : from(start), to(end)
	{
	}

	constexpr Ranger(const size_t length, T* start) noexcept : from(start), to(start + length)
	{
	}
	
	constexpr T* begin() noexcept { return from; }
	constexpr T* end() noexcept { return to; }
};