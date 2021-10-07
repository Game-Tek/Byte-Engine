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

template<typename E, typename T>
constexpr E GetType();

class InputManager : public Object
{
public:
	enum class Type {
		NONE, BOOL, CHAR, LINEAR, VECTOR2D, VECTOR3D, COLOR, QUATERNION
	};

	union Datatypes {
		Datatypes() : Color(0, 0, 0, 0) {}
		Datatypes(bool b) : Action(b) {}
		Datatypes(char32_t c) : Unicode(c) {}
		Datatypes(float32 f) : Linear(f) {}
		Datatypes(GTSL::Vector2 v) : Vector2D(v) {}
		Datatypes(GTSL::Vector3 v) : Vector3D(v) {}
		Datatypes(GTSL::RGBA r) : Color(r) {}
		Datatypes(GTSL::Quaternion q) : Quaternion(q) {}

		bool Action; char32_t Unicode; float32 Linear;
		GTSL::Vector2 Vector2D; GTSL::Vector3 Vector3D;
		GTSL::RGBA Color;
		GTSL::Quaternion Quaternion;
	};

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
	using CharacterInputEvent = InputEvent<char32_t>;
	using Vector2DInputEvent = InputEvent<GTSL::Vector2>;
	using Vector3DInputEvent = InputEvent<GTSL::Vector3>;
	using QuaternionInputEvent = InputEvent<GTSL::Quaternion>;

	InputManager();
	~InputManager();

	InputLayerHandle RegisterInputLayer(const Id inputLayerName) {
		uint32 index = inputLayers.GetLength();
		return InputLayerHandle(index);
	}
	
	InputDeviceHandle RegisterInputDevice(Id inputDeviceName) {
		auto index = inputDevices.GetLength();
		auto& inputDevice = inputDevices.EmplaceBack();
		inputDevice.Name = inputDeviceName;
		auto deviceIndex = inputDevice.ActiveIndeces.GetLength();
		inputDevice.ActiveIndeces.EmplaceBack(0);
		return InputDeviceHandle(index, deviceIndex);
	}

	void UnregisterInputDevice(InputDeviceHandle inputDeviceHandle) {
		if (inputDeviceHandle.DeviceHandle + 1u > inputDevices.GetLength()) { BE_LOG_WARNING(u8"Tried to unregister an input source but it wasn't registered."); return; }
		inputDevices.Pop(inputDeviceHandle.DeviceHandle);
	}

	void RegisterInputSource(InputDeviceHandle, Id inputSourceName, Type type) {
		auto inputSource = inputSources.TryEmplace(inputSourceName);

		if (!inputSource.State()) {
			BE_LOG_WARNING(u8"Tried to register input source ", GTSL::StringView(inputSourceName), u8" but it was already registered.");
			return;
		}

		inputSource.Get().SourceType = type;
		//inputSource.Get().Pos = inputSourcesRecords.GetLength();

		switch (type) {
		case Type::BOOL: {
			//*inputSourcesRecords.AllocateStructure<bool>() = false;
		}
		case Type::CHAR: {
			//*inputSourcesRecords.AllocateStructure<char32_t>() = 0;
		}
		case Type::LINEAR: {
			//*inputSourcesRecords.AllocateStructure<float32>() = 0.0f;
		}
		case Type::VECTOR2D: {
			//inputSourcesRecords.AllocateStructure<GTSL::Vector2>();
		}
		case Type::VECTOR3D: {
			//inputSourcesRecords.AllocateStructure<GTSL::Vector3>();
		}
		}
	}

	void RegisterInputSources(InputDeviceHandle, GTSL::Range<const Id*> inputSourceNames, Type type) {
		for (auto& e : inputSourceNames) {
			auto inputSource = inputSources.TryEmplace(e);

			if (!inputSource.State()) {
				BE_LOG_WARNING(u8"Tried to register input source ", GTSL::StringView(e), u8" but it was already registered.");
				return;
			}

			inputSource.Get().SourceType = type;
			//inputSource.Get().Pos = inputSourcesRecords.GetLength();

			//switch (type) {
			//case InputSource::BOOL: {
			//	*inputSourcesRecords.AllocateStructure<bool>() = false;
			//}
			//case InputSource::Type::CHAR: {
			//	*inputSourcesRecords.AllocateStructure<char32_t>() = 0;
			//}
			//case InputSource::Type::LINEAR: {
			//	*inputSourcesRecords.AllocateStructure<float32>() = 0.0f;
			//}
			//case InputSource::Type::VECTOR2D: {
			//	inputSourcesRecords.AllocateStructure<GTSL::Vector2>();
			//}
			//case InputSource::Type::VECTOR3D: {
			//	inputSourcesRecords.AllocateStructure<GTSL::Vector3>();
			//}
			//}
		}
	}

	struct Action {
		Action(const Id inputSource, const Id actionName) : InputSourceName(inputSource), ActionName(actionName), ActionType(Type::NONE) {}

		template<typename T>
		Action(const Id inputSource, const Id actionName, T val) : InputSourceName(inputSource), ActionName(actionName), Datatype(val), ActionType(GetType<Type, T>()) {}

		Id InputSourceName, ActionName;
		Datatypes Datatype;
		Type ActionType;
	};

	template<typename T>
	void SubscribeToInputEvent(Id eventName, GTSL::Range<const Action*> inputSourceNames, DynamicTaskHandle<T> function) {
		auto inputEventIndex = inputEvents.GetLength();
		auto& inputEvent = inputEvents.EmplaceBack();

		inputEvent.Handle = function.Reference;
		inputEvent.EventType = GetType<Type, typename T::type>();

		for (const auto& action : inputSourceNames) {
			auto inputSource = inputSources.TryGet(action.InputSourceName);

			if (inputSource.State()) {
				inputEvent.InputSources.Emplace(action.InputSourceName).TargetValue = action.Datatype;

				inputSource.Get().BoundInputEvents.EmplaceBack(inputEventIndex);
			} else {
				BE_LOG_WARNING(u8"Failed to register ", GTSL::StringView(action.ActionName), u8" action, input source ", GTSL::StringView(action.InputSourceName), u8" was not registered. Cannot create an action event which depends on a non existant input source, make sure the input source is registered before registering this input event");
			}
		}
	}

	template<typename T>
	void RecordInputSource(InputDeviceHandle deviceIndex, Id eventName, T newValue)
	{
		if (!inputSources.Find(eventName)) { BE_LOG_WARNING(u8"Tried to record ", GTSL::StringView(eventName), u8" which is not registered as an input source."); return; }
		if (inputSources[eventName].SourceType != GetType<Type, T>()) { BE_LOG_WARNING(u8"Tried to record ", GTSL::StringView(eventName), u8" but the input source's type does not match the type of the data being supplied."); return; }

		inputSourceRecords.EmplaceBack(deviceIndex, eventName, newValue);
	}

	ActionInputEvent::type GetActionInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return inputSources[eventName].LastValue.Action;
	}

	CharacterInputEvent::type GetCharacterInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return inputSources[eventName].LastValue.Unicode;
	}

	LinearInputEvent::type GetLinearInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return inputSources[eventName].LastValue.Linear;
	}

	Vector2DInputEvent::type GetVector2DInputSourceValue(InputDeviceHandle deviceHandle, Id eventName) const {
		return inputSources[eventName].LastValue.Vector2D;
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
	struct InputSource {		
		GTSL::Microseconds LastTime;
		Datatypes LastValue;
		float32 Threshold = 0.95f, DeadZone = 0.1f;

		Type SourceType;
		
		GTSL::StaticVector<uint32, 8> BoundInputEvents;

		InputSource() = default;

		template<typename T>
		InputSource(const T lstValue, const GTSL::Microseconds lstTime) : LastTime(lstTime), LastValue(lstValue)
		{
		}
	};

	struct InputEventData {
		Type EventType;
		uint32 Handle = ~0U;

		struct Action {
			Datatypes TargetValue;
			uint32_t StackEntry = ~0U;
		};
		GTSL::StaticMap<Id, Action, 4> InputSources{ 4, 0.75f };

		GTSL::StaticVector<Datatypes, 4> Stack;
	};
	GTSL::Vector<InputEventData, BE::PersistentAllocatorReference> inputEvents;

	struct InputDevice {
		Id Name;
		GTSL::StaticVector<uint32, 8> ActiveIndeces;
		GTSL::StaticMap<Id, Datatypes, 8> Parameters;
	};
	GTSL::Vector<InputDevice, BE::PAR> inputDevices;
	GTSL::HashMap<Id, InputSource, BE::PersistentAllocatorReference> inputSources;
	
	/**
	* \brief Defines an InputSourceRecord which is record of the value the physical input source(keyboard, mouse, VR controller, etc) it is associated to had when it was triggered.
	* This can be a boolean value(on, off) triggered by a keyboard key, mouse click, etc;
	* a linear value(X) triggered by a gamepad trigger, slider value, etc;
	* a 3D value(X, Y, Z) triggered by a VR controller move, hand tracker move, etc;
	* and a Quaternion value(X, Y, Z, Q)(rotation) triggered by a VR controller rotation change, phone orientation change, etc.
	*/
	struct InputSourceRecord {
		InputDeviceHandle DeviceIndex;

		/**
		 * \brief Name of the input source which caused the input source event.
		 */
		Id InputSource;

		Datatypes NewValue;

		InputSourceRecord() = default;

		template<typename T>
		InputSourceRecord(InputDeviceHandle deviceIndex, const Id name, T newValue) : InputSource(name), DeviceIndex(deviceIndex), NewValue(newValue)
		{
		}
	};

	GTSL::Vector<InputSourceRecord, BE::PersistentAllocatorReference> inputSourceRecords;

	InputLayerHandle activeInputLayer;
	GTSL::SemiVector<uint32, 8, BE::PAR> inputLayers;

	void updateInput(ApplicationManager* application_manager, GTSL::Microseconds time) {
		for (auto& record : inputSourceRecords) {
			auto& inputSource = inputSources[record.InputSource];

			for (const auto bie : inputSource.BoundInputEvents) {
				auto& inputEventData = inputEvents[bie];

				if (inputEventData.Handle != ~0U) {
					switch (inputEventData.EventType) {
					case Type::BOOL: {
						bool newValue = false, oldValue = false;

						switch (inputSource.SourceType) {
						case Type::NONE: break;
						case Type::BOOL: {
							newValue = record.NewValue.Action;
							oldValue = inputSource.LastValue.Action;
							break;
						}
						case Type::CHAR: break;
						case Type::LINEAR: {
							const bool wasPressed = inputSource.LastValue.Linear >= inputSource.Threshold;

							if (record.NewValue.Linear >= inputSource.Threshold) { //if is pressed
								if (!wasPressed) { //and wasn't pressed
									oldValue = false;
									newValue = true;
								}
							}
							else { //isn't pressed
								if (wasPressed && record.NewValue.Linear <= inputSource.Threshold - 0.10f) {
									oldValue = true;
									newValue = false;
								}
							}
							break;
						}
						case Type::VECTOR2D: break;
						case Type::VECTOR3D: break;
						case Type::COLOR: break;
						case Type::QUATERNION: break;
						}

						if (oldValue != newValue) {
							application_manager->AddStoredDynamicTask(DynamicTaskHandle<InputEvent<bool>>(inputEventData.Handle), InputEvent(record.DeviceIndex, record.InputSource, inputSource.LastTime, newValue, oldValue));
						}

						break;
					}
					case Type::CHAR: {
						application_manager->AddStoredDynamicTask(DynamicTaskHandle<InputEvent<char32_t>>(inputEventData.Handle), InputEvent(record.DeviceIndex, record.InputSource, inputSource.LastTime, record.NewValue.Unicode, U'0'));
						break;
					}
					case Type::LINEAR: {
						float32 newVal = 0.0f, oldVal = 0.0f;

						switch (inputSource.SourceType) {
						case Type::BOOL: {
							auto& action = inputEventData.InputSources[record.InputSource];

							if (record.NewValue.Action) {
								newVal = action.TargetValue.Linear;

								if (action.StackEntry == ~0U) {
									action.StackEntry = inputEventData.Stack.GetLength();
									auto& stackEntry = inputEventData.Stack.EmplaceBack();
									stackEntry.Linear = newVal;
								}
							} else {
								if (action.StackEntry != ~0U) {
									inputEventData.Stack.Pop(action.StackEntry);

									for(auto& e : inputEventData.InputSources) {
										if(e.StackEntry > action.StackEntry and e.StackEntry != ~0U) {
											--e.StackEntry;
										}
									}

									action.StackEntry = ~0U;
								}

								if (inputEventData.Stack.GetLength()) {
									newVal = inputEventData.Stack.back().Linear;
								}
							}
							break;
						}
						case Type::LINEAR: {
							newVal = record.NewValue.Linear;
							oldVal = inputSource.LastValue.Linear;
							break;
						}
						}

						application_manager->AddStoredDynamicTask(DynamicTaskHandle<InputEvent<float32>>(inputEventData.Handle), InputEvent(record.DeviceIndex, record.InputSource, inputSource.LastTime, newVal, oldVal));

						break;
					}
					case Type::VECTOR2D: {
						GTSL::Vector2 newValue, oldValue;

						switch (inputSource.SourceType) {
						case Type::VECTOR2D: {
							newValue = record.NewValue.Vector2D;
							break;
						}
						}

						if (GTSL::Math::MagnitudeGreater(newValue, GTSL::Vector2(inputSource.Threshold))) {
							application_manager->AddStoredDynamicTask(DynamicTaskHandle<InputEvent<GTSL::Vector2>>(inputEventData.Handle), InputEvent(record.DeviceIndex, record.InputSource, inputSource.LastTime, newValue, oldValue));
						}
						break;
					}
					default: __debugbreak();
					}
				}
			}

			inputSource.LastValue = record.NewValue;
			inputSource.LastTime = time;
		}

		inputSourceRecords.Resize(0);
	}
};

template<>
constexpr InputManager::Type GetType<InputManager::Type, bool>() { return InputManager::Type::BOOL; }

template<>
constexpr InputManager::Type GetType<InputManager::Type, char32_t>() { return InputManager::Type::CHAR; }

template<>
constexpr InputManager::Type GetType<InputManager::Type, float32>() { return InputManager::Type::LINEAR; }

template<>
constexpr InputManager::Type GetType<InputManager::Type, GTSL::Vector2>() { return InputManager::Type::VECTOR2D; }

template<>
constexpr InputManager::Type GetType<InputManager::Type, GTSL::Vector3>() { return InputManager::Type::VECTOR3D; }

template<>
constexpr InputManager::Type GetType<InputManager::Type, GTSL::RGBA>() { return InputManager::Type::COLOR; }

template<>
constexpr InputManager::Type GetType<InputManager::Type, GTSL::Quaternion>() { return InputManager::Type::QUATERNION; }