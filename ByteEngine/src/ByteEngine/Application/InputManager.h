#pragma once

#include "ByteEngine/Object.h"

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/Time.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Quaternion.h>
#include <GTSL/Math/Vector2.h>
#include <GTSL/Math/Vector3.h>


#include "ByteEngine/Core.h"

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
		GTSL::Id64 Name;
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
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }
	
	void RegisterActionInputSource(GTSL::Id64 inputSourceName);
	void RegisterCharacterInputSource(GTSL::Id64 inputSourceName);
	void RegisterLinearInputSource(GTSL::Id64 inputSourceName);
	void Register2DInputSource(GTSL::Id64 inputSourceName);

	void RegisterActionInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames, GTSL::Delegate<void(ActionInputEvent)> function);
	void RegisterCharacterInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames, GTSL::Delegate<void(CharacterInputEvent)> function);
	void RegisterLinearInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames, GTSL::Delegate<void(LinearInputEvent)> function);
	void Register2DInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames, GTSL::Delegate<void(Vector2DInputEvent)> function);
	
	void RecordActionInputSource(GTSL::Id64 inputSourceName, const ActionInputEvent::type& newValue);
	void RecordCharacterInputSource(GTSL::Id64 inputSourceName, const CharacterInputEvent::type& newValue);
	void RecordLinearInputSource(GTSL::Id64 inputSourceName, const float32 newValue);
	void Record2DInputSource(GTSL::Id64 inputSourceName, const Vector2DInputEvent::type& newValue);

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
	std::unordered_map<GTSL::Id64::HashType, ActionInputSourceData> actionInputSourcesToActionInputEvents;
	
	using LinearInputSourceData = InputSourceData<LinearInputEvent>;
	std::unordered_map<GTSL::Id64::HashType, LinearInputSourceData> linearInputSourcesToLinearInputEvents;

	using CharacterInputSourceData = InputSourceData<CharacterInputEvent>;
	std::unordered_map<GTSL::Id64::HashType, CharacterInputSourceData> characterInputSourcesToCharacterInputEvents;
	
	using Vector2DInputSourceData = InputSourceData<Vector2DInputEvent>;
	std::unordered_map<GTSL::Id64::HashType, Vector2DInputSourceData> vector2dInputSourceEventsToVector2DInputEvents;
	
	using Vector3DInputSourceData = InputSourceData<Vector3DInputEvent>;
	std::unordered_map<GTSL::Id64::HashType, Vector3DInputSourceData> vector3dInputSourcesToVector3DInputEvents;

	using QuaternionInputSourceData = InputSourceData<QuaternionInputEvent>;
	std::unordered_map<GTSL::Id64::HashType, QuaternionInputSourceData> quaternionInputSourcesToQuaternionInputEvents;


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
		GTSL::Id64 Name;

		typename T::type NewValue;

		InputSourceRecord() = default;
		
		InputSourceRecord(const GTSL::Id64 name, decltype(NewValue) newValue) : Name(name), NewValue(newValue)
		{
		}
	};
	
	/**
	 * \brief InputSourceRecord for a 2D value(X, Y) triggered by a gamepad stick, mouse move, etc.
	 */
	GTSL::Vector<InputSourceRecord<ActionInputEvent>> actionInputSourceRecords;
	GTSL::Vector<InputSourceRecord<LinearInputEvent>> linearInputSourceRecords;
	GTSL::Vector<InputSourceRecord<CharacterInputEvent>> characterInputSourceRecords;
	GTSL::Vector<InputSourceRecord<Vector2DInputEvent>> vector2DInputSourceRecords;
	GTSL::Vector<InputSourceRecord<Vector3DInputEvent>> vector3DInputSourceRecords;
	GTSL::Vector<InputSourceRecord<QuaternionInputEvent>> quaternionInputSourceRecords;
};
