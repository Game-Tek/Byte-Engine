#pragma once

#include "Core.h"

#include "Event.h"

#include "Math\Vector2.h"

GS_STRUCT MouseMovedEvent : public Event
{
	Vector2 MouseOffset;

	MouseMovedEvent()
	{
	}

	MouseMovedEvent(const Vector2 & MouseOffset) : MouseOffset(MouseOffset)
	{
	}
};