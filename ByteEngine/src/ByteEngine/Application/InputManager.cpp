#include "InputManager.h"

#include <GTSL/Math/Vector2.h>

#include "Application.h"
#include "ByteEngine/Debug/Assert.h"

InputManager::InputManager() : actionInputSourcesToActionInputEvents(128, 0.2f, GetPersistentAllocator()), characterInputSourcesToCharacterInputEvents(2, GetPersistentAllocator()),
linearInputSourcesToLinearInputEvents(32, 0.2f, GetPersistentAllocator()), vector2dInputSourceEventsToVector2DInputEvents(32, 0.2f, GetPersistentAllocator())
{
	actionInputSourceRecords.Initialize(10, GetPersistentAllocator());
	characterInputSourceRecords.Initialize(10, GetPersistentAllocator());
	linearInputSourceRecords.Initialize(10, GetPersistentAllocator());
	vector2DInputSourceRecords.Initialize(10, GetPersistentAllocator());
}

InputManager::~InputManager()
{
	actionInputSourceRecords.Free(GetPersistentAllocator());
	characterInputSourceRecords.Free(GetPersistentAllocator());
	linearInputSourceRecords.Free(GetPersistentAllocator());
	vector2DInputSourceRecords.Free(GetPersistentAllocator());

	actionInputSourcesToActionInputEvents.Free(GetPersistentAllocator());
	characterInputSourcesToCharacterInputEvents.Free(GetPersistentAllocator());
	linearInputSourcesToLinearInputEvents.Free(GetPersistentAllocator());
	vector2dInputSourceEventsToVector2DInputEvents.Free(GetPersistentAllocator());
}

void InputManager::Update()
{
	GTSL::Microseconds current_time{ 0 };

	updateInput(actionInputSourceRecords, actionInputSourcesToActionInputEvents, current_time);
	updateInput(characterInputSourceRecords, characterInputSourcesToCharacterInputEvents, current_time);
	updateInput(linearInputSourceRecords, linearInputSourcesToLinearInputEvents, current_time);
	updateInput(vector2DInputSourceRecords, vector2dInputSourceEventsToVector2DInputEvents, current_time);
}

void InputManager::RegisterActionInputSource(const GTSL::Id64 inputSourceName)
{
	actionInputSourcesToActionInputEvents.Emplace(GetPersistentAllocator(), inputSourceName, ActionInputSourceData());
}

void InputManager::RegisterCharacterInputSource(const GTSL::Id64 inputSourceName)
{
	characterInputSourcesToCharacterInputEvents.Emplace(GetPersistentAllocator(), inputSourceName, CharacterInputSourceData());
}

void InputManager::RegisterLinearInputSource(const GTSL::Id64 inputSourceName)
{
	linearInputSourcesToLinearInputEvents.Emplace(GetPersistentAllocator(), inputSourceName, LinearInputSourceData());
}

void InputManager::Register2DInputSource(const GTSL::Id64 inputSourceName)
{
	vector2dInputSourceEventsToVector2DInputEvents.Emplace(GetPersistentAllocator(), inputSourceName, Vector2DInputSourceData());
}

void InputManager::RegisterActionInputEvent(GTSL::Id64 actionName, GTSL::Ranger<const GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(ActionInputEvent)> function)
{
#ifdef BE_DEBUG
	//for (auto& e : inputSourceNames) { BE_ASSERT(actionInputSourcesToActionInputEvents.At(e) != actionInputSourcesToActionInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif
	
	for (const GTSL::Id64& e : inputSourceNames) { actionInputSourcesToActionInputEvents.At(e) = ActionInputSourceData(function, {}, {}); }
}

void InputManager::RegisterCharacterInputEvent(GTSL::Id64 actionName, GTSL::Ranger<const GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(CharacterInputEvent)> function)
{
#ifdef BE_DEBUG
	//for (auto& e : inputSourceNames) { BE_ASSERT(characterInputSourcesToCharacterInputEvents.find(e) != characterInputSourcesToCharacterInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for (const auto& e : inputSourceNames) { characterInputSourcesToCharacterInputEvents.At(e) = CharacterInputSourceData(function, {}, {}); }
}

void InputManager::RegisterLinearInputEvent(GTSL::Id64 actionName, GTSL::Ranger<const GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(LinearInputEvent)> function)
{
#ifdef BE_DEBUG
	//for (auto& e : inputSourceNames) { BE_ASSERT(linearInputSourcesToLinearInputEvents.find(e) != linearInputSourcesToLinearInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for (const auto& e : inputSourceNames) { linearInputSourcesToLinearInputEvents.At(e) = LinearInputSourceData(function, {}, {}); }
}

void InputManager::Register2DInputEvent(GTSL::Id64 actionName, GTSL::Ranger<const GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(Vector2DInputEvent)> function)
{
#ifdef BE_DEBUG
	//for (auto& e : inputSourceNames) { BE_ASSERT(vector2dInputSourceEventsToVector2DInputEvents.find(e) != vector2dInputSourceEventsToVector2DInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for(const auto& e : inputSourceNames) { vector2dInputSourceEventsToVector2DInputEvents.At(e) = Vector2DInputSourceData(function, {}, {}); }
}

void InputManager::RecordActionInputSource(GTSL::Id64 inputSourceName, ActionInputEvent::type newValue)
{
	actionInputSourceRecords.EmplaceBack(GetPersistentAllocator(), inputSourceName, newValue );
}

void InputManager::RecordCharacterInputSource(GTSL::Id64 inputSourceName, CharacterInputEvent::type newValue)
{
	characterInputSourceRecords.EmplaceBack(GetPersistentAllocator(), inputSourceName, newValue);
}

void InputManager::RecordLinearInputSource(GTSL::Id64 inputSourceName, const LinearInputEvent::type newValue)
{
	linearInputSourceRecords.EmplaceBack(GetPersistentAllocator(), inputSourceName, newValue);
}

void InputManager::Record2DInputSource(const GTSL::Id64 inputSourceName, Vector2DInputEvent::type newValue)
{
	vector2DInputSourceRecords.EmplaceBack(GetPersistentAllocator(), inputSourceName, newValue);
}
