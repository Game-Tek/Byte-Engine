#include "InputManager.h"

#include <GTSL/Window.h>
#include <GTSL/Math/Vector2.h>

#include "Application.h"
#include "ByteEngine/Debug/Assert.h"

static BE::PersistentAllocatorReference allocator_reference("Input Manager");

InputManager::InputManager()
{
	input2DSourceRecords.Initialize(50, &allocator_reference);
}

void InputManager::Update()
{
	GTSL::Microseconds current_time{ 0 };
	
	for(auto& record2D : input2DSourceRecords)
	{
		auto& inputSource = vector2dInputSourceEventsToVector2DInputEvents.at(record2D.Name);
		inputSource.Function(Vector2DInputEvent{ record2D.Name, current_time - inputSource.LastTime, record2D.NewValue, record2D.NewValue - inputSource.LastValue });
	}

	input2DSourceRecords.Resize(0);
}

void InputManager::Register2DInputSource(const GTSL::Id64 inputSourceName)
{
	vector2dInputSourceEventsToVector2DInputEvents.insert({ inputSourceName, {} });
}

void InputManager::Register2DInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames)
{
#ifdef BE_DEBUG
	for (auto& e : inputSourceNames) { if(vector2dInputSourceEventsToVector2DInputEvents.find(e) == vector2dInputSourceEventsToVector2DInputEvents.end()) BE_ASSERT(false, "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for(auto& e : inputSourceNames) { vector2dInputSourceEventsToVector2DInputEvents.insert({ e, {} }); }
}

void InputManager::Record2DInputSource(const GTSL::Id64 inputSourceName, const GTSL::Vector2& newValue)
{
	//axis2DInputSourceRecords.PushBack({ inputSourceName , newValue, newValue });
}
