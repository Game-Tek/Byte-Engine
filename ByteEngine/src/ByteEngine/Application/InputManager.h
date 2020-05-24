#pragma once

#include "ByteEngine/Core.h"

#include "ByteEngine/Object.h"

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/Pair.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vector2.h>

namespace GTSL {
	class Window;
}

class InputManager : public Object
{
	std::unordered_map<GTSL::Id64::HashType, GTSL::Delegate<void(bool)>> buttons;
	std::unordered_map<GTSL::Id64::HashType, GTSL::Delegate<void(GTSL::Vector2, GTSL::Vector2)>> axis;

	GTSL::Vector<GTSL::Pair<GTSL::Id64, bool>> buttonEvents;

	struct AxisEvent
	{
		GTSL::Id64 Id;
		GTSL::Vector2 NewValue;
		GTSL::Vector2 Delta;
	};
	GTSL::Vector<AxisEvent> axisEvents;

public:
	InputManager();
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }

	void BindWindow(GTSL::Window* window);

	void RegisterKeyAction(GTSL::Id64 key, GTSL::Delegate<void(bool)> del);
	void RegisterAxisAction(GTSL::Id64 key, GTSL::Delegate<void(GTSL::Vector2, GTSL::Vector2)> del);
	
	void SignalAxis(GTSL::Id64 name, GTSL::Vector2 a, GTSL::Vector2 b);

	void SignalButtonPress(GTSL::Id64 key, bool cond);

	void Update();
};
