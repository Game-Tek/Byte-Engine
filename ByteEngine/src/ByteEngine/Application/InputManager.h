#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/StaticMap.hpp>
#include <GTSL/Time.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Quaternion.h>
#include <GTSL/Math/Vector2.h>
#include <GTSL/Math/Vector3.h>

#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Logger.h"

#include "ByteEngine/Handle.hpp"

namespace GTSL {
	class Window;
}

MAKE_HANDLE(uint8, InputDevice)

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
		Id Name, SourceDevice;
		InputDeviceHandle DeviceIndex;
		GTSL::Microseconds LastEventTime;
		T Value, LastValue;
	};

	using ActionInputEvent = InputEvent<bool>;
	using LinearInputEvent = InputEvent<float32>;
	using CharacterInputEvent = InputEvent<uint32>;
	using Vector2DInputEvent = InputEvent<GTSL::Vector2>;
	using Vector3DInputEvent = InputEvent<GTSL::Vector3>;
	using QuaternionInputEvent = InputEvent<GTSL::Quaternion>;
	
	InputManager();
	~InputManager();

	InputDeviceHandle RegisterInputDevice(Id name) {		
		auto index = deviceProperties.Emplace(name).EmplaceBack();
		return InputDeviceHandle(index);
	}

	void UnregisterInputDevice(Id name, InputDeviceHandle inputDeviceHandle) {
		if (!deviceProperties.Find(name)) { BE_LOG_WARNING("Tried to unregister input source ", name.GetString(), " but it wasn't registered."); return; }
		deviceProperties.Remove(name);
	}
	
	void RegisterActionInputSource(InputDeviceHandle, Id inputSourceName)
	{
		auto result = actionInputSourcesToActionInputEvents.TryEmplace(inputSourceName, ActionInputSourceData());

		if (!result.State()) {
			BE_LOG_WARNING("Tried to register action input source ", inputSourceName.GetString(), " but it was already registered.");
		}
	}
	
	void RegisterCharacterInputSource(InputDeviceHandle, Id inputSourceName)
	{
		auto result = characterInputSourcesToCharacterInputEvents.TryEmplace(inputSourceName, CharacterInputSourceData());
		
		if(!result.State()) {
			BE_LOG_WARNING("Tried to register character input source ", inputSourceName.GetString(), " but it was already registered.");
		}
	}
	
	void RegisterLinearInputSource(InputDeviceHandle, Id inputSourceName)
	{
		auto result = linearInputSourcesToLinearInputEvents.TryEmplace(inputSourceName, LinearInputSourceData());

		if (!result.State()) {
			BE_LOG_WARNING("Tried to register linear input source ", inputSourceName.GetString(), " but it was already registered.");
		}
	}
	
	void Register2DInputSource(InputDeviceHandle, Id inputSourceName)
	{
		auto result = vector2dInputSourceEventsToVector2DInputEvents.TryEmplace(inputSourceName, Vector2DInputSourceData());

		if (!result.State()) {
			BE_LOG_WARNING("Tried to register 2D input source ", inputSourceName.GetString(), " but it was already registered.");
		}
	}

	void RegisterActionInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, GTSL::Delegate<void(ActionInputEvent)> function)
	{
		for (const auto& e : inputSourceNames)
		{
			auto res = actionInputSourcesToActionInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = ActionInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event");
			}
		}
	}
	
	void RegisterCharacterInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, GTSL::Delegate<void(CharacterInputEvent)> function)
	{
		for (const auto& e : inputSourceNames)
		{
			auto res = characterInputSourcesToCharacterInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = CharacterInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event");
			}
		}
	}
	
	void RegisterLinearInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, GTSL::Delegate<void(LinearInputEvent)> function)
	{
		for (const auto& e : inputSourceNames)
		{
			auto res = linearInputSourcesToLinearInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = LinearInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event");
			}
		}
	}
	
	void Register2DInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, GTSL::Delegate<void(Vector2DInputEvent)> function)
	{
		for (const auto& e : inputSourceNames)
		{
			auto res = vector2dInputSourceEventsToVector2DInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = Vector2DInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register InputEvent, dependent Input Source was not registered. Cannot create an Input Event which depends on a non existant Input Source, make sure the Input Source is registered before registering this Input Event");
			}
		}
	}
	
	void RecordActionInputSource(Id sourceDevice, InputDeviceHandle deviceIndex, Id eventName, ActionInputEvent::type newValue)
	{
		if (!actionInputSourcesToActionInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as an action input source."); return; }
		actionInputSourceRecords.EmplaceBack(sourceDevice, deviceIndex, eventName, newValue);
	}
	
	void RecordCharacterInputSource(Id sourceDevice, InputDeviceHandle deviceIndex, Id eventName, CharacterInputEvent::type newValue)
	{
		if (!characterInputSourcesToCharacterInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as a character input source."); return; }
		characterInputSourceRecords.EmplaceBack(sourceDevice, deviceIndex, eventName, newValue);
	}
	
	void RecordLinearInputSource(Id sourceDevice, InputDeviceHandle deviceIndex, Id eventName, LinearInputEvent::type newValue)
	{
		if (!linearInputSourcesToLinearInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as a linear input source."); return; }
		linearInputSourceRecords.EmplaceBack(sourceDevice, deviceIndex, eventName, newValue);
	}
	
	void Record2DInputSource(Id sourceDevice, InputDeviceHandle deviceIndex, Id eventName, Vector2DInputEvent::type newValue)
	{
		if (!vector2dInputSourceEventsToVector2DInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as a vector 2d input source."); return; }
		vector2DInputSourceRecords.EmplaceBack(sourceDevice, deviceIndex, eventName, newValue);
	}

	void SetDeviceProperty(Id device, uint8 deviceIndex, float32 value)
	{
		deviceProperties.At(device)[deviceIndex] = value;
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

	GTSL::StaticMap<Id, GTSL::Array<float32, 8>, 8> deviceProperties;
	
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
		Id Source, Name; InputDeviceHandle DeviceIndex;

		typename T::type NewValue;

		InputSourceRecord() = default;
		
		InputSourceRecord(const Id source, InputDeviceHandle deviceIndex, const Id name, decltype(NewValue) newValue) : Source(source), Name(name), DeviceIndex(deviceIndex), NewValue(newValue)
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

			if (inputSource.Function) { inputSource.Function({ record.Name, record.Source, record.DeviceIndex, inputSource.LastTime, record.NewValue, inputSource.LastValue }); }

			inputSource.LastValue = record.NewValue;
			inputSource.LastTime = time;
		}

		records.ResizeDown(0);
	}
};
