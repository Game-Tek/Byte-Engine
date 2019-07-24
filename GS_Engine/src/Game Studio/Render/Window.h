#pragma once

#include "Core.h"

#include "Extent.h"
#include "FString.h"

#include "../Math/Vector2.h"

#include "../InputEnums.h"

enum class WindowFit : uint8
{
	NORMAL, MAXIMIZED, FULLSCREEN
};

GS_STRUCT WindowCreateInfo
{
	Extent2D Extent;
	String Name;
	WindowFit WindowType = WindowFit::NORMAL;
};

	GS_CLASS Window
	{
	protected:
		Extent2D Extent;
		Vector2 MousePosition;
		bool ShouldClose = false;

		KeyState Keys[MAX_KEYBOARD_KEYS];
	public:
		Window(Extent2D _Extent) : Extent(_Extent)
		{
		}

		static Window* CreateGSWindow(const WindowCreateInfo& _WCI);

		virtual void Update() {};

		[[nodiscard]] const Extent2D& GetWindowExtent() const { return Extent; }
		[[nodiscard]] const Vector2& GetMousePosition() const { return MousePosition; }
		INLINE bool GetShouldClose() const { return ShouldClose; }
		INLINE float GetAspectRatio() const { return SCAST(float, Extent.Width) / SCAST(float, Extent.Height); }
	};