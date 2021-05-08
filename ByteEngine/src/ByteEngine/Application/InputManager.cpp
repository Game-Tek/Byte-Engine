#include "InputManager.h"

#include <GTSL/Math/Vectors.h>


#include "Application.h"
#include "ByteEngine/Debug/Logger.h"

InputManager::InputManager() : Object("InputManager"), actionInputSourcesToActionInputEvents(128, 0.2f, GetPersistentAllocator()),
                               characterInputSourcesToCharacterInputEvents(2, GetPersistentAllocator()),
                               linearInputSourcesToLinearInputEvents(32, 0.2f, GetPersistentAllocator()),
                               vector2dInputSourceEventsToVector2DInputEvents(32, 0.2f, GetPersistentAllocator()),

                               actionInputSourceRecords(10, GetPersistentAllocator()),
                               characterInputSourceRecords(10, GetPersistentAllocator()),
                               linearInputSourceRecords(10, GetPersistentAllocator()),
                               vector2DInputSourceRecords(10, GetPersistentAllocator())
{
}

InputManager::~InputManager()
{
}

void InputManager::Update()
{
	GTSL::Microseconds current_time{ BE::Application::Get()->GetClock()->GetElapsedTime() };

	updateInput(actionInputSourceRecords, actionInputSourcesToActionInputEvents, current_time);
	updateInput(characterInputSourceRecords, characterInputSourcesToCharacterInputEvents, current_time);
	updateInput(linearInputSourceRecords, linearInputSourcesToLinearInputEvents, current_time);
	updateInput(vector2DInputSourceRecords, vector2dInputSourceEventsToVector2DInputEvents, current_time);
}