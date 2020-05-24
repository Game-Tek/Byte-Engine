#include "InputManager.h"

#include <GTSL/Window.h>
#include <GTSL/Math/Vector2.h>

#include "Application.h"

static BE::PersistentAllocatorReference allocator_reference("Input Manager");

void InputManager::SignalAxis(GTSL::Id64 name, GTSL::Vector2 a, GTSL::Vector2 b)
{
	axisEvents.PushBack({name, a, b});
}

void InputManager::SignalButtonPress(const GTSL::Id64 key, const bool cond)
{
	buttonEvents.PushBack(GTSL::Pair<GTSL::Id64, bool>(key, cond));
}

void InputManager::Update()
{
	for (auto& e : buttonEvents)
	{
		buttons.at(e.First.GetID())(e.Second);
	}

	for (auto& e : axisEvents)
	{
		axis.at(e.Id)(e.NewValue, e.Delta);
	}


	buttonEvents.Resize(0);
	axisEvents.Resize(0);
}

InputManager::InputManager()
{
	buttonEvents.Initialize(50, &allocator_reference);
	axisEvents.Initialize(50, &allocator_reference);
}

void InputManager::BindWindow(GTSL::Window* window)
{

}

void InputManager::RegisterKeyAction(GTSL::Id64 key, GTSL::Delegate<void(bool)> del)
{
	buttons.insert({key, del});
}

void InputManager::RegisterAxisAction(GTSL::Id64 key, GTSL::Delegate<void(GTSL::Vector2, GTSL::Vector2)> del)	
{
	axis.insert({ key, del });
}
