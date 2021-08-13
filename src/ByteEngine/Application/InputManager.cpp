#include "InputManager.h"

#include "Application.h"
#include "ByteEngine/Debug/Logger.h"

InputManager::InputManager() : Object(u8"InputManager"), actionInputSourcesToActionInputEvents(128, 0.2f, GetPersistentAllocator()),
								characterInputSourcesToCharacterInputEvents(2, GetPersistentAllocator()),
								linearInputSourcesToLinearInputEvents(32, 0.2f, GetPersistentAllocator()),
								vector2dInputSourceEventsToVector2DInputEvents(32, 0.2f, GetPersistentAllocator()),
								vector3dInputSourcesToVector3DInputEvents(32, 0.2f, GetPersistentAllocator()),
								quaternionInputSourcesToQuaternionInputEvents(16, 0.2f, GetPersistentAllocator()),

								actionInputSourceRecords(10, GetPersistentAllocator()),
								characterInputSourceRecords(10, GetPersistentAllocator()),
								linearInputSourceRecords(10, GetPersistentAllocator()),
								vector2DInputSourceRecords(10, GetPersistentAllocator()),

								inputLayers(4, { GetPersistentAllocator() })
{
}

InputManager::~InputManager()
{
}

void InputManager::Update()
{
	GTSL::Microseconds current_time{ BE::Application::Get()->GetClock()->GetElapsedTime() };

	auto* applicationManager = BE::Application::Get()->GetGameInstance();

	updateInput(applicationManager, actionInputSourceRecords, actionInputSourcesToActionInputEvents, current_time);
	updateInput(applicationManager, characterInputSourceRecords, characterInputSourcesToCharacterInputEvents, current_time);
	updateInput(applicationManager, linearInputSourceRecords, linearInputSourcesToLinearInputEvents, current_time);
	updateInput(applicationManager, vector2DInputSourceRecords, vector2dInputSourceEventsToVector2DInputEvents, current_time);
}