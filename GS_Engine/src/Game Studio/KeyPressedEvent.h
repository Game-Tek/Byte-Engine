#pragma once

#include "Event.h"

enum Key
{
	W = 0, A, S, D
};

GS_STRUCT KeyPressedEvent : public Event
{
	Key PressedKey = W;

	KeyPressedEvent()
	{
	}

	KeyPressedEvent(Key InKey) : PressedKey(InKey)
	{
	}

	~KeyPressedEvent()
	{
	}
};