#pragma once

#include <unordered_map>
#include <GTSL/Id.h>


#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"
#include <GTSL/Delegate.hpp>
#include <GTSL/Pair.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vector2.h>

class InputManager : public Object
{
	//std::unordered_map<GTSL::Id16, Delegate<void(float)>> axisActions;
	//std::unordered_map<GTSL::Id16, Delegate<void(bool)>> buttonActions;
	//
	std::unordered_map<GTSL::Id64::HashType, Delegate<void(bool)>> buttons;
	std::unordered_map<GTSL::Id64::HashType, Delegate<void(GTSL::Vector2)>> axis;

	GTSL::Vector<GTSL::Pair<GTSL::Id64, bool>> buttonEvents;
	
public:
	InputManager() = default;
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }

	void RegisterKeyAction(GTSL::Id64 key, Delegate<void(bool)> del)
	{
		buttons.insert({ key, del });
	}
	
	void SignalButtonPress(const GTSL::Id64 key, const bool cond)
	{
		buttonEvents.PushBack(GTSL::Pair<GTSL::Id64, bool>(key, cond));
	}

	void RegisterAxisEvent(bool cond)
	{
	}
	
	void Update()
	{
		for (auto& e : buttonEvents)
		{
			buttons.at(e.First.GetID())(e.Second);
		}
		
		
		buttonEvents.Resize(0);
	}
};
