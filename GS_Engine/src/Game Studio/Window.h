#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "ImageSize.h"

GS_CLASS Window : public ESystem
{
public:
	Window(const ImageSize& _WindowSize, const char * WindowName);
	~Window();

	void OnUpdate() override;

	INLINE ImageSize GetWindowSize() const { return WindowSize; }
	INLINE uint16 GetWindowWidth() const { return WindowSize.Width; }
	INLINE uint16 GetWindowHeight() const { return WindowSize.Height; }

	INLINE float GetAspectRatio() const { return static_cast<float>(WindowSize.Width) / static_cast<float>(WindowSize.Height); }

	void ResizeWindow(const uint16 WWidth, const uint16 WHeight);
	void ResizeWindow(const ImageSize & _WindowSize);

protected:
	ImageSize WindowSize;
};