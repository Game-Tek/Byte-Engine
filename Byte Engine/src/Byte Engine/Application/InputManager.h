#pragma once

#include "Byte Engine/Object.h"

class InputManager : public Object
{
	//std::unordered_map<GTSL::Id16, Delegate<void(float)>> axisActions;
	//std::unordered_map<GTSL::Id16, Delegate<void(bool)>>	buttonActions;

public:
	InputManager() = default;
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }
	void Update()
	{
		
	}
};
