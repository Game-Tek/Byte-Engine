#pragma once

/**
 * \brief Contains a bool which flips it's state every time the object(FlipFlop) is evaluated as a bool. Useful for setting or keeping track of sticky states.
 */
class FlipFlop
{
	bool state = true;

public:
	FlipFlop() = default;

	FlipFlop(const bool state) : state(state)
	{
	}

	FlipFlop(const FlipFlop& other) = default;

	~FlipFlop() = default;

	operator bool() { state = !state; return state; }

	[[nodiscard]] bool GetState() const { return state; }
	void SetState(const bool newState) { state = newState; }
	void FlipState() { state = !state; }
};
