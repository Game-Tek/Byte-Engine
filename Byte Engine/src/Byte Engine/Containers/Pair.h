#pragma once

template <typename _A, typename _B>
struct Pair
{
	_A First;
	_B Second;

	Pair() = default;

	Pair(const _A& _First, const _B& _Second) : First(_First), Second(_Second)
	{
	}
};
