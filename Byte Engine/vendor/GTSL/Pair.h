#pragma once

template <typename A, typename B>
struct Pair
{
	A First;
	B Second;

	Pair() = default;

	constexpr Pair(const A& first, const B& second) noexcept : First(first), Second(second)
	{
	}
};
