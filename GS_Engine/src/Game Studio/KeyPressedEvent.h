#pragma once

#include "Event.h"

#include "InputEnums.h"

GS_STRUCT KeyPressedEvent : public Event
{
	KeyboardKeys PressedKey = W;

	KeyPressedEvent()
	{
	}

	KeyPressedEvent(KeyboardKeys InKey) : PressedKey(InKey)
	{
	}

	~KeyPressedEvent()
	{
	}
};