#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/RGB.h>
#include <GTSL/Time.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Quaternion.h>
#include <GTSL/Math/Vectors.h>

#include "ByteEngine/Id.h"

#include "ByteEngine/Debug/Logger.h"
#include "ByteEngine/Game/ApplicationManager.h"

namespace GTSL {
	class Window;
}

struct InputDeviceHandle
{
	uint8 DeviceHandle, DeviceIndex;
};

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
		InputDeviceHandle DeviceIndex;
		Id InputSource;
		GTSL::Microseconds LastEventTime;
		T Value, LastValue;

		InputEvent(InputDeviceHandle device_handle, Id inputSource, GTSL::Microseconds lastEvent, T val, T lastVal) : DeviceIndex(device_handle), InputSource(inputSource), LastEventTime(lastEvent), Value(val), LastValue(lastVal) {}
	};

	MAKE_HANDLE(uint32, InputLayer)
	
	using ActionInputEvent = InputEvent<bool>;
	using LinearInputEvent = InputEvent<float32>;
	using CharacterInputEvent = InputEvent<uint32>;
	using Vector2DInputEvent = InputEvent<GTSL::Vector2>;
	using Vector3DInputEvent = InputEvent<GTSL::Vector3>;
	using QuaternionInputEvent = InputEvent<GTSL::Quaternion>;
	
	InputManager();
	~InputManager();

	InputLayerHandle RegisterInputLayer(const Id name)
	{
		uint32 index = inputLayers.GetLength();
		return InputLayerHandle(index);
	}
	
	InputDeviceHandle RegisterInputDevice(Id name) {
		auto index = inputDevices.GetLength();
		auto& inputDevice = inputDevices.EmplaceBack();
		inputDevice.Name = name;
		auto deviceIndex = inputDevice.ActiveIndeces.GetLength();
		inputDevice.ActiveIndeces.EmplaceBack(0);
		return InputDeviceHandle(index, deviceIndex);
	}

	void UnregisterInputDevice(InputDeviceHandle inputDeviceHandle) {
		if (inputDeviceHandle.DeviceHandle + 1u > inputDevices.GetLength()) { BE_LOG_WARNING("Tried to unregister an input source but it wasn't registered."); return; }
		inputDevices.Pop(inputDeviceHandle.DeviceHandle);
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

	void SubscribeToInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, DynamicTaskHandle<ActionInputEvent> function) {
		for (const auto& e : inputSourceNames)
		{
			auto res = actionInputSourcesToActionInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = ActionInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to bind action input event ", actionName.GetString(), " to ", e.GetString(), ". As that input source was not registered.");
			}
		}
	}
	
	void SubscribeToInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, DynamicTaskHandle<CharacterInputEvent> function) {
		for (const auto& e : inputSourceNames)
		{
			auto res = characterInputSourcesToCharacterInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = CharacterInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register ", actionName.GetString(), " character input event, dependent input source was not registered. Cannot create an input event which depends on a non existant input source, make sure the input source is registered before registering this input event");
			}
		}
	}
	
	void SubscribeToInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, DynamicTaskHandle<LinearInputEvent> function) {
		for (const auto& e : inputSourceNames)
		{
			auto res = linearInputSourcesToLinearInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = LinearInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register ", actionName.GetString(), " linear input event, dependent input source was not registered. Cannot create an input event which depends on a non existant input source, make sure the input source is registered before registering this input event");
			}
		}
	}
	
	void SubscribeToInputEvent(Id actionName, GTSL::Range<const Id*> inputSourceNames, DynamicTaskHandle<Vector2DInputEvent> function) {
		for (const auto& e : inputSourceNames)
		{
			auto res = vector2dInputSourceEventsToVector2DInputEvents.TryGet(e);
			if (res.State()) {
				res.Get() = Vector2DInputSourceData(function, {}, {});
			}
			else {
				BE_LOG_WARNING("Failed to register ", actionName.GetString(), " 2D input event, dependent input source was not registered. Cannot create an input event which depends on a non existant input source, make sure the input source is registered before registering this input event");
			}
		}
	}
	
	void RecordInputSource(InputDeviceHandle deviceIndex, Id eventName, ActionInputEvent::type newValue)
	{
		if (!actionInputSourcesToActionInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as an action input source."); return; }
		actionInputSourceRecords.EmplaceBack(deviceIndex, eventName, newValue);
	}
	
	void RecordInputSource(InputDeviceHandle deviceIndex, Id eventName, CharacterInputEvent::type newValue)
	{
		if (!characterInputSourcesToCharacterInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as a character input source."); return; }
		characterInputSourceRecords.EmplaceBack(deviceIndex, eventName, newValue);
	}
	
	void RecordInputSource(InputDeviceHandle deviceIndex, Id eventName, LinearInputEvent::type newValue)
	{
		if (!linearInputSourcesToLinearInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as a linear input source."); return; }
		linearInputSourceRecords.EmplaceBack(deviceIndex, eventName, newValue);
	}
	
	void RecordInputSource(InputDeviceHandle deviceIndex, Id eventName, Vector2DInputEvent::type newValue)
	{
		if (!vector2dInputSourceEventsToVector2DInputEvents.Find(eventName)) { BE_LOG_WARNING("Tried to record ", eventName.GetString(), " which is not registered as a vector 2d input source."); return; }
		vector2DInputSourceRecords.EmplaceBack(deviceIndex, eventName, newValue);
	}

	ActionInputEvent::type GetActionInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return actionInputSourcesToActionInputEvents[eventName].LastValue;
	}

	CharacterInputEvent::type GetCharacterInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return characterInputSourcesToCharacterInputEvents[eventName].LastValue;
	}

	LinearInputEvent::type GetLinearInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return linearInputSourcesToLinearInputEvents[eventName].LastValue;
	}

	Vector2DInputEvent::type GetVector2DInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return vector2dInputSourceEventsToVector2DInputEvents[eventName].LastValue;
	}
	
	void Update();
	
	void SetInputDeviceParameter(InputDeviceHandle deviceHandle, Id parameterName, float32 value) {
		inputDevices[deviceHandle.DeviceHandle].Parameters.At(parameterName).Linear = value;
	}

	void SetInputDeviceParameter(InputDeviceHandle deviceHandle, Id parameterName, GTSL::RGBA value) {
		inputDevices[deviceHandle.DeviceHandle].Parameters.At(parameterName).Color = value;
	}

	[[nodiscard]] float32 GetInputDeviceParameter(InputDeviceHandle inputDeviceHandle, Id parameterName) const {
		return inputDevices[inputDeviceHandle.DeviceHandle].Parameters.At(parameterName).Linear;
	}

	void RegisterInputDeviceParameter(InputDeviceHandle inputDeviceHandle, Id parameterName) {
		inputDevices[inputDeviceHandle.DeviceHandle].Parameters.Emplace(parameterName);
	}

protected:
	template<typename T>
	struct InputSourceData
	{
		DynamicTaskHandle<T> Task;
		typename T::type LastValue;
		GTSL::Microseconds LastTime;

		InputSourceData() = default;

		InputSourceData(const DynamicTaskHandle<T> func, const typename T::type lstValue, const GTSL::Microseconds lstTime) : Task(func), LastValue(lstValue), LastTime(lstTime)
		{
		}
	};

	struct InputDevice {
		Id Name;
		GTSL::StaticVector<uint32, 8> ActiveIndeces;

		union Datatypes {
			Datatypes() {}

			bool Action; uint32 Unicode; float32 Linear;
			GTSL::Vector2 Vector2D; GTSL::Vector3 Vector3D;
			GTSL::RGBA Color;
			GTSL::Quaternion Quaternion;
		};

		GTSL::StaticMap<Id, Datatypes, 8> Parameters;
	};
	GTSL::StaticVector<InputDevice, 16> inputDevices;
	
	using ActionInputSourceData = InputSourceData<ActionInputEvent>;
	GTSL::HashMap<Id, ActionInputSourceData, BE::PersistentAllocatorReference> actionInputSourcesToActionInputEvents;

	using CharacterInputSourceData = InputSourceData<CharacterInputEvent>;
	GTSL::HashMap<Id, CharacterInputSourceData, BE::PersistentAllocatorReference> characterInputSourcesToCharacterInputEvents;
	
	using LinearInputSourceData = InputSourceData<LinearInputEvent>;
	GTSL::HashMap<Id, LinearInputSourceData, BE::PersistentAllocatorReference> linearInputSourcesToLinearInputEvents;
	
	using Vector2DInputSourceData = InputSourceData<Vector2DInputEvent>;
	GTSL::HashMap<Id, Vector2DInputSourceData, BE::PersistentAllocatorReference> vector2dInputSourceEventsToVector2DInputEvents;
	
	using Vector3DInputSourceData = InputSourceData<Vector3DInputEvent>;
	GTSL::HashMap<Id, Vector3DInputSourceData, BE::PersistentAllocatorReference> vector3dInputSourcesToVector3DInputEvents;

	using QuaternionInputSourceData = InputSourceData<QuaternionInputEvent>;
	GTSL::HashMap<Id, QuaternionInputSourceData, BE::PersistentAllocatorReference> quaternionInputSourcesToQuaternionInputEvents;
	
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
		Id InputSource; InputDeviceHandle DeviceIndex;

		typename T::type NewValue;

		using III = T;

		InputSourceRecord() = default;
		
		InputSourceRecord(InputDeviceHandle deviceIndex, const Id name, decltype(NewValue) newValue) : InputSource(name), DeviceIndex(deviceIndex), NewValue(newValue)
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

	InputLayerHandle activeInputLayer;
	GTSL::SemiVector<uint32, 8, BE::PAR> inputLayers;
	
	template<typename A, typename B>
	static void updateInput(ApplicationManager* application_manager, GTSL::Vector<A, BE::PersistentAllocatorReference>& records, GTSL::HashMap<Id, B, BE::PersistentAllocatorReference>& map, GTSL::Microseconds time)
	{
		for (auto& record : records)
		{
			auto& inputSource = map.At(record.InputSource);

			if (inputSource.Task) { application_manager->AddStoredDynamicTask(inputSource.Task, A::III(record.DeviceIndex, record.InputSource, inputSource.LastTime, record.NewValue, inputSource.LastValue)); }

			inputSource.LastValue = record.NewValue;
			inputSource.LastTime = time;
		}

		records.Resize(0);
	}
};
