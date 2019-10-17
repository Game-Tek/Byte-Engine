#pragma once

#include "Core.h"

template<typename A, typename B>
class GS_API Pair
{
public:
	A First;
	B Second;

	Pair() = default;

	Pair(const A& _A, const B& _B) : First(_A), Second(_B)
	{
	}
};