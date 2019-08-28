#pragma once

#include "Core.h"

GS_CLASS FlipFlop
{
	bool State = false;

public:
	FlipFlop() = default;

	explicit FlipFlop(const bool _State) : State(_State)
	{
	}

	FlipFlop(const FlipFlop& _Other) = default;

	~FlipFlop() = default;

	operator bool()
	{
		State = !State;
		return State;
	}

	[[nodiscard]] bool GetState() const { return State; }
};