#include "InputManager.h"

#include <GTSL/Window.h>
#include <GTSL/Math/Vector2.h>

#include "Application.h"
#include "ByteEngine/Debug/Assert.h"

static BE::PersistentAllocatorReference allocator_reference("Input Manager");

InputManager::InputManager()
{
	actionInputSourceRecords.Initialize(10, &allocator_reference);
	characterInputSourceRecords.Initialize(10, &allocator_reference);
	linearInputSourceRecords.Initialize(10, &allocator_reference);
	vector2DInputSourceRecords.Initialize(10, &allocator_reference);
}

void InputManager::Update()
{
	GTSL::Microseconds current_time{ 0 };
	
	for(auto& action_record : actionInputSourceRecords)
	{
		auto& inputSource = actionInputSourcesToActionInputEvents.at(action_record.Name);
		
		if(inputSource.Function)
		{
			inputSource.Function({ action_record.Name, inputSource.LastTime, action_record.NewValue, inputSource.LastValue });
		}
		
		inputSource.LastValue = action_record.NewValue;
		inputSource.LastTime = current_time;
	}

	for(auto& character_record : characterInputSourceRecords)
	{
		auto& inputSource = characterInputSourcesToCharacterInputEvents.at(character_record.Name);
		
		if(inputSource.Function)
		{
			inputSource.Function({ character_record.Name, inputSource.LastTime, character_record.NewValue, inputSource.LastValue });
		}
		
		inputSource.LastValue = character_record.NewValue;
		inputSource.LastTime = current_time;
	}

	for(auto& linear_record : linearInputSourceRecords)
	{
		auto& inputSource = linearInputSourcesToLinearInputEvents.at(linear_record.Name);
		
		if(inputSource.Function)
		{
			inputSource.Function({ linear_record.Name, inputSource.LastTime, linear_record.NewValue, inputSource.LastValue });
		}
		
		inputSource.LastValue = linear_record.NewValue;
		inputSource.LastTime = current_time;
	}
	
	for(auto& record2D : vector2DInputSourceRecords)
	{
		auto& inputSource = vector2dInputSourceEventsToVector2DInputEvents.at(record2D.Name);
		
		if(inputSource.Function)
		{
			inputSource.Function({ record2D.Name, inputSource.LastTime, record2D.NewValue, inputSource.LastValue });
		}
		
		inputSource.LastValue = record2D.NewValue;
		inputSource.LastTime = current_time;
	}

	actionInputSourceRecords.Resize(0);
	characterInputSourceRecords.Resize(0);
	linearInputSourceRecords.Resize(0);
	vector2DInputSourceRecords.Resize(0);
}

void InputManager::RegisterActionInputSource(const GTSL::Id64 inputSourceName)
{
	actionInputSourcesToActionInputEvents.emplace(inputSourceName, ActionInputSourceData());
}

void InputManager::RegisterCharacterInputSource(GTSL::Id64 inputSourceName)
{
	characterInputSourcesToCharacterInputEvents.emplace(inputSourceName, CharacterInputSourceData());
}

void InputManager::RegisterLinearInputSource(GTSL::Id64 inputSourceName)
{
	linearInputSourcesToLinearInputEvents.emplace(inputSourceName, LinearInputSourceData());
}

void InputManager::Register2DInputSource(const GTSL::Id64 inputSourceName)
{
	vector2dInputSourceEventsToVector2DInputEvents.emplace(inputSourceName, Vector2DInputSourceData());
}

void InputManager::RegisterActionInputEvent(GTSL::Id64 actionName, GTSL::Ranger<const GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(ActionInputEvent)> function)
{
#ifdef BE_DEBUG
	for (auto& e : inputSourceNames) { BE_ASSERT(actionInputSourcesToActionInputEvents.find(e) != actionInputSourcesToActionInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif
	
	for (auto& e : inputSourceNames)
	{
		actionInputSourcesToActionInputEvents.at(e) = ActionInputSourceData(function, {}, {});
	}
}

void InputManager::RegisterCharacterInputEvent(GTSL::Id64 actionName, GTSL::Ranger<const GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(CharacterInputEvent)> function)
{
#ifdef BE_DEBUG
	for (auto& e : inputSourceNames) { BE_ASSERT(characterInputSourcesToCharacterInputEvents.find(e) != characterInputSourcesToCharacterInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for (auto& e : inputSourceNames)
	{
		characterInputSourcesToCharacterInputEvents.at(e) = CharacterInputSourceData(function, {}, {});
	}
}

void InputManager::RegisterLinearInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(LinearInputEvent)> function)
{
#ifdef BE_DEBUG
	for (auto& e : inputSourceNames) { BE_ASSERT(linearInputSourcesToLinearInputEvents.find(e) != linearInputSourcesToLinearInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for (auto& e : inputSourceNames)
	{
		linearInputSourcesToLinearInputEvents.at(e) = LinearInputSourceData(function, {}, {});
	}
}

void InputManager::Register2DInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames, const GTSL::Delegate<void(Vector2DInputEvent)> function)
{
#ifdef BE_DEBUG
	for (auto& e : inputSourceNames) { BE_ASSERT(vector2dInputSourceEventsToVector2DInputEvents.find(e) != vector2dInputSourceEventsToVector2DInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

	for(auto& e : inputSourceNames)
	{
		vector2dInputSourceEventsToVector2DInputEvents.at(e) = Vector2DInputSourceData(function, {}, {});
	}
}

void InputManager::RecordActionInputSource(GTSL::Id64 inputSourceName, ActionInputEvent::type newValue)
{
	actionInputSourceRecords.EmplaceBack(inputSourceName, newValue );
}

void InputManager::RecordCharacterInputSource(GTSL::Id64 inputSourceName, CharacterInputEvent::type newValue)
{
	characterInputSourceRecords.EmplaceBack(inputSourceName, newValue);
}

void InputManager::RecordLinearInputSource(GTSL::Id64 inputSourceName, const LinearInputEvent::type newValue)
{
	linearInputSourceRecords.EmplaceBack(inputSourceName, newValue);
}

void InputManager::Record2DInputSource(const GTSL::Id64 inputSourceName, Vector2DInputEvent::type newValue)
{
	vector2DInputSourceRecords.EmplaceBack(inputSourceName, newValue);
}
