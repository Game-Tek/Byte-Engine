#pragma once

#include "Core.h"

template<typename A, typename B>
class GS_API Tuple
{
public:
	A First;
	B Second;

	Tuple() = default;

	Tuple(const A& _A, const B& _B) : First(_A), Second(_B)
	{
	}
};