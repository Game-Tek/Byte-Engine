#include "InputManager.h"

#include "Application.h"
#include "ByteEngine/Debug/Logger.h"

InputManager::InputManager() : Object(u8"InputManager"),
								inputEvents(64, GetPersistentAllocator()),
								inputDevices(8, GetPersistentAllocator()),
								inputSources(128, 0.2f, GetPersistentAllocator()),
								inputSourceRecords(8, GetPersistentAllocator()),
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

	updateInput(applicationManager, current_time);
}