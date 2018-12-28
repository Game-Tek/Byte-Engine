#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "glad.h"
#include "glfw3.h"

#include "Window.h"

GS_CLASS Renderer : ESystem
{
public:
	Renderer(Window * WD);
	~Renderer();

	void OnUpdate(float DeltaTime) override;

private:
	Window * WindowInstanceRef;
};

