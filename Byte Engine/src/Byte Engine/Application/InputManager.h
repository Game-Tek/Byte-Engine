#pragma once

#include "Core.h"

#include "Object.h"

#include "Math/Vector2.h"
#include "Containers/Array.hpp"
#include "Input/MouseState.h"
#include "Input/InputEnums.h"
#include "Input/JoystickState.h"
#include "Containers/Id.h"
#include "Containers/FVector.hpp"
#include "Utility/Delegate.h"
#include <map>
#include <unordered_map>
#include "Core/Window.h"

namespace RAPI
{
	class Window;
}

class InputManager : public Object
{
	std::unordered_map<Id16, Delegate<void(float)>> axisActions;
	std::unordered_map<Id16, Delegate<void(bool)>>	buttonActions;

public:
	InputManager() = default;
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }
	void Update();
};
