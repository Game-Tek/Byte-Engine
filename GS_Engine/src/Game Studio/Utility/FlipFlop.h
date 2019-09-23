#pragma once

#include "Core.h"

//Contains a bool which flips it's state every time the object(FlipFlop) is evaluated as a bool. Useful for setting or keeping track of sticky states.
class GS_API FlipFlop
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
	void SetState(const bool _State) { State = _State; }
	void FlipState() { State = !State; }
};
