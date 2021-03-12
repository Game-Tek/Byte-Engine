#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Time.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Quaternion.h>
#include <GTSL/Math/Vector2.h>
#include <GTSL/Math/Vector3.h>

#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Logger.h"

namespace GTSL {
	class Window;
}

class InputManager : public Object
{
public:

	/**
	 * \brief Defines an input event which is a named event that is triggered when one of the InputSourceEvents that it is bound to occurs.
	 */
	template<typename T>
	struct InputEvent
	{
		using type = T;
		Id Name;
		GTSL::Microseconds LastEventTime;
		T Value;
		T LastValue;
	};

	using ActionInputEvent = InputEvent<bool>;
	using LinearInputEvent = InputEvent<float32>;
	using CharacterInputEvent = InputEvent<uint32>;
	using Vector2DInputEvent = InputEvent<GTSL::Vector2>;
	using Vector3DInputEvent = InputEvent<GTSL::Vector3>;
	using QuaternionInputEvent = InputEvent<GTSL::Quaternion>;
	
	InputManager();
	~InputManager();
	
	void RegisterActionInputSource(Id inputSourceName)
	{
		if constexpr (_DEBUG) {
			if (actionInputSourcesToActionInputEvents.Find(inputSourceName))
			{
				BE_LOG_ERROR("Tried to register action input source ", inputSourceName.GetString(), " but it was already registered.", BE::FIX_OR_CRASH_STRING);
				return;
			}
		}

		actionInputSourcesToActionInputEvents.Emplace(inputSourceName, ActionInputSourceData());
	}
	
	void RegisterCharacterInputSource(Id inputSourceName)
	{
		if constexpr (_DEBUG) {
			if (characterInputSourcesToCharacterInputEvents.Find(inputSourceName))
			{
				BE_LOG_ERROR("Tried to register character input source ", inputSourceName.GetString(), " but it was already registered.", BE::FIX_OR_CRASH_STRING);
				return;
			}
		}

		characterInputSourcesToCharacterInputEvents.Emplace(inputSourceName, CharacterInputSourceData());
	}
	
	void RegisterLinearInputSource(Id inputSourceName)
	{
		if constexpr (_DEBUG) {
			if (linearInputSourcesToLinearInputEvents.Find(inputSourceName))
			{
				BE_LOG_ERROR("Tried to register linear input source ", inputSourceName.GetString(), " but it was already registered.", BE::FIX_OR_CRASH_STRING);
				return;
			}
		}

		linearInputSourcesToLinearInputEvents.Emplace(inputSourceName, LinearInputSourceData());
	}
	
	void Register2DInputSource(Id inputSourceName)
	{
		if constexpr (_DEBUG) {
			if (vector2dInputSourceEventsToVector2DInputEvents.Find(inputSourceName))
			{
				BE_LOG_ERROR("Tried to register 2D input source ", inputSourceName.GetString(), " but it was already registered.", BE::FIX_OR_CRASH_STRING);
				return;
			}
		}

		vector2dInputSourceEventsToVector2DInputEvents.Emplace(inputSourceName, Vector2DInputSourceData());
	}

	void RegisterActionInputEvent(Id actionName, GTSL::Range<const GTSL::Id64*> inputSourceNames, GTSL::Delegate<void(ActionInputEvent)> function)
	{
#ifdef BE_DEBUG
		//for (auto& e : inputSourceNames) { BE_ASSERT(actionInputSourcesToActionInputEvents.At(e) != actionInputSourcesToActionInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

		for (const auto& e : inputSourceNames) { actionInputSourcesToActionInputEvents.At(e) = ActionInputSourceData(function, {}, {}); }
	}
	
	void RegisterCharacterInputEvent(Id actionName, GTSL::Range<const GTSL::Id64*> inputSourceNames, GTSL::Delegate<void(CharacterInputEvent)> function)
	{
#ifdef BE_DEBUG
		//for (auto& e : inputSourceNames) { BE_ASSERT(characterInputSourcesToCharacterInputEvents.find(e) != characterInputSourcesToCharacterInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

		for (const auto& e : inputSourceNames) { characterInputSourcesToCharacterInputEvents.At(e) = CharacterInputSourceData(function, {}, {}); }
	}
	
	void RegisterLinearInputEvent(Id actionName, GTSL::Range<const GTSL::Id64*> inputSourceNames, GTSL::Delegate<void(LinearInputEvent)> function)
	{
#ifdef BE_DEBUG
		//for (auto& e : inputSourceNames) { BE_ASSERT(linearInputSourcesToLinearInputEvents.find(e) != linearInputSourcesToLinearInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

		for (const auto& e : inputSourceNames) { linearInputSourcesToLinearInputEvents.At(e) = LinearInputSourceData(function, {}, {}); }
	}
	
	void Register2DInputEvent(Id actionName, GTSL::Range<const GTSL::Id64*> inputSourceNames, GTSL::Delegate<void(Vector2DInputEvent)> function)
	{
#ifdef BE_DEBUG
		//for (auto& e : inputSourceNames) { BE_ASSERT(vector2dInputSourceEventsToVector2DInputEvents.find(e) != vector2dInputSourceEventsToVector2DInputEvents.end(), "Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event"); }
#endif

		for (const auto& e : inputSourceNames) { vector2dInputSourceEventsToVector2DInputEvents.At(e) = Vector2DInputSourceData(function, {}, {}); }
	}
	
	void RecordActionInputSource(Id inputSourceName, ActionInputEvent::type newValue)
	{
		if (!actionInputSourcesToActionInputEvents.Find(inputSourceName)) { BE_LOG_WARNING("Tried to record ", inputSourceName.GetString(), " which is not registered as an action input source."); return; }
		actionInputSourceRecords.EmplaceBack(inputSourceName, newValue);
	}
	
	void RecordCharacterInputSource(Id inputSourceName, CharacterInputEvent::type newValue)
	{
		if (!characterInputSourcesToCharacterInputEvents.Find(inputSourceName)) { BE_LOG_WARNING("Tried to record ", inputSourceName.GetString(), " which is not registered as a character input source."); return; }
		characterInputSourceRecords.EmplaceBack(inputSourceName, newValue);
	}
	
	void RecordLinearInputSource(Id inputSourceName, LinearInputEvent::type newValue)
	{
		if (!linearInputSourcesToLinearInputEvents.Find(inputSourceName)) { BE_LOG_WARNING("Tried to record ", inputSourceName.GetString(), " which is not registered as a linear input source."); return; }
		linearInputSourceRecords.EmplaceBack(inputSourceName, newValue);
	}
	
	void Record2DInputSource(Id inputSourceName, Vector2DInputEvent::type newValue)
	{
		if (!vector2dInputSourceEventsToVector2DInputEvents.Find(inputSourceName)) { BE_LOG_WARNING("Tried to record ", inputSourceName.GetString(), " which is not registered as a vector 2d input source."); return; }
		vector2DInputSourceRecords.EmplaceBack(inputSourceName, newValue);
	}

	void Update();	
	
protected:
	template<typename T>
	struct InputSourceData
	{
		GTSL::Delegate<void(T)> Function;
		typename T::type LastValue;
		GTSL::Microseconds LastTime;

		InputSourceData() = default;

		InputSourceData(const GTSL::Delegate<void(T)> func, const typename T::type lstValue, const GTSL::Microseconds lstTime) : Function(func), LastValue(lstValue), LastTime(lstTime)
		{
		}
	};
	
	using ActionInputSourceData = InputSourceData<ActionInputEvent>;
	GTSL::FlatHashMap<Id, ActionInputSourceData, BE::PersistentAllocatorReference> actionInputSourcesToActionInputEvents;

	using CharacterInputSourceData = InputSourceData<CharacterInputEvent>;
	GTSL::FlatHashMap<Id, CharacterInputSourceData, BE::PersistentAllocatorReference> characterInputSourcesToCharacterInputEvents;
	
	using LinearInputSourceData = InputSourceData<LinearInputEvent>;
	GTSL::FlatHashMap<Id, LinearInputSourceData, BE::PersistentAllocatorReference> linearInputSourcesToLinearInputEvents;
	
	using Vector2DInputSourceData = InputSourceData<Vector2DInputEvent>;
	GTSL::FlatHashMap<Id, Vector2DInputSourceData, BE::PersistentAllocatorReference> vector2dInputSourceEventsToVector2DInputEvents;
	
	using Vector3DInputSourceData = InputSourceData<Vector3DInputEvent>;
	GTSL::FlatHashMap<Id, Vector3DInputSourceData, BE::PersistentAllocatorReference> vector3dInputSourcesToVector3DInputEvents;

	using QuaternionInputSourceData = InputSourceData<QuaternionInputEvent>;
	GTSL::FlatHashMap<Id, QuaternionInputSourceData, BE::PersistentAllocatorReference> quaternionInputSourcesToQuaternionInputEvents;
	
	/**
	* \brief Defines an InputSourceRecord which is record of the value the physical input source(keyboard, mouse, VR controller, etc) it is associated to had when it was triggered.
	* This can be a boolean value(on, off) triggered by a keyboard key, mouse click, etc;
	* a linear value(X) triggered by a gamepad trigger, slider value, etc;
	* a 3D value(X, Y, Z) triggered by a VR controller move, hand tracker move, etc;
	* and a Quaternion value(X, Y, Z, Q)(rotation) triggered by a VR controller rotation change, phone orientation change, etc.
	*/
	template<typename T>
	struct InputSourceRecord
	{
		/**
		 * \brief Name of the input source which caused the input source event.
		 */
		Id Name;

		typename T::type NewValue;

		InputSourceRecord() = default;
		
		InputSourceRecord(const Id name, decltype(NewValue) newValue) : Name(name), NewValue(newValue)
		{
		}
	};
	
	/**
	 * \brief InputSourceRecord for a 2D value(X, Y) triggered by a gamepad stick, mouse move, etc.
	 */
	GTSL::Vector<InputSourceRecord<ActionInputEvent>, BE::PersistentAllocatorReference> actionInputSourceRecords;
	GTSL::Vector<InputSourceRecord<CharacterInputEvent>, BE::PersistentAllocatorReference> characterInputSourceRecords;
	GTSL::Vector<InputSourceRecord<LinearInputEvent>, BE::PersistentAllocatorReference> linearInputSourceRecords;
	GTSL::Vector<InputSourceRecord<Vector2DInputEvent>, BE::PersistentAllocatorReference> vector2DInputSourceRecords;
	//GTSL::Vector<InputSourceRecord<Vector3DInputEvent>> vector3DInputSourceRecords;
	//GTSL::Vector<InputSourceRecord<QuaternionInputEvent>> quaternionInputSourceRecords;
	//
	template<typename A, typename B>
	static void updateInput(GTSL::Vector<A, BE::PersistentAllocatorReference>& records, GTSL::FlatHashMap<Id, B, BE::PersistentAllocatorReference>& map, GTSL::Microseconds time)
	{
		for (auto& record : records)
		{
			auto& inputSource = map.At(record.Name);

			if (inputSource.Function) { inputSource.Function({ record.Name, inputSource.LastTime, record.NewValue, inputSource.LastValue }); }

			inputSource.LastValue = record.NewValue;
			inputSource.LastTime = time;
		}

		records.ResizeDown(0);
	}
};
