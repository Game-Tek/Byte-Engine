#pragma once

#include "Object.h"

#include <GTSL/Array.hpp>
#include <GTSL/Id.h>
#include <GTSL/Delegate.h>
#include <unordered_map>

namespace GAL
{
	class Window;
}

class InputManager : public Object
{
	std::unordered_map<GTSL::Id16, Delegate<void(float)>> axisActions;
	std::unordered_map<GTSL::Id16, Delegate<void(bool)>>	buttonActions;

public:
	InputManager() = default;
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }
	void Update();
};
