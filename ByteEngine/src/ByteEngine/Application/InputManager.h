#pragma once

#include "ByteEngine/Core.h"

#include "ByteEngine/Object.h"

#include <unordered_map>
#include <GTSL/Id.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/Pair.h>
#include <GTSL/Time.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vector2.h>

namespace GTSL {
	class Window;
}

class InputManager : public Object
{
public:
	/**
	* \brief Defines an InputSourceRecord which is record of the value the physical input source(keyboard, mouse, VR controller, etc) it is associated to had when it was triggered.
	* This can be a boolean value(on, off) triggered by a keyboard key, mouse click, etc;
	* a linear value(X) triggered by a gamepad trigger, slider value, etc;
	* a 2D value(X, Y) triggered by a gamepad stick, mouse move;
	* a 3D value(X, Y, Z) triggered by a VR controller move, hand tracker move, etc;
	* and a Quaternion value(X, Y, Z, Q)(rotation) triggered by a VR controller rotation change, phone orientation change, etc.
	*/
	struct InputSourceRecord
	{
		/**
		 * \brief Name of the input source which changed caused the 2D axis input source event,
		 */
		GTSL::Id64 Name;
	};

	struct Axis2DRecord : InputSourceRecord
	{
		GTSL::Vector2 NewValue;
	};

	/**
	 * \brief Defines an input event which is a named event that is triggered when one of the InputSourceEvents that it is bound to occurs.
	 */
	struct InputEvent
	{
		GTSL::Id64 Name;
		GTSL::Microseconds TimeSinceLastEvent;
	};

	struct Vector2DInputEvent : InputEvent
	{
		GTSL::Vector2 Value;
		GTSL::Vector2 Delta;
	};
	
	
	InputManager();
	~InputManager() = default;

	[[nodiscard]] const char* GetName() const override { return "Input Manager"; }

	void Register2DInputSource(GTSL::Id64 inputSourceName);

	void Register2DInputEvent(GTSL::Id64 actionName, GTSL::Ranger<GTSL::Id64> inputSourceNames);

	void Record2DInputSource(GTSL::Id64 inputSourceName, const GTSL::Vector2& newValue);

	void Update();

protected:
	//std::unordered_map<GTSL::Id64::HashType, GTSL::Vector<GTSL::Id64::HashType>> actionInputSourcesToActionInputEvents;
	//std::unordered_map<GTSL::Id64::HashType, GTSL::Vector<GTSL::Id64::HashType>> linearInputSourcesToLinearInputEvents;
	//
	struct Vector2DInputSourceData
	{
		GTSL::Delegate<void(Vector2DInputEvent)> Function;
		GTSL::Vector2 LastValue;
		GTSL::Microseconds LastTime;
	};
	std::unordered_map<GTSL::Id64::HashType, Vector2DInputSourceData> vector2dInputSourceEventsToVector2DInputEvents;
	//std::unordered_map<GTSL::Id64::HashType, GTSL::Vector<GTSL::Id64::HashType>> vector3dInputSourcesToVector3DInputEvents;
	//std::unordered_map<GTSL::Id64::HashType, GTSL::Vector<GTSL::Id64::HashType>> quaternionInputSourcesToQuaternionInputEvents;
	//
	GTSL::Vector<Axis2DRecord> input2DSourceRecords;
};
