#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "W:/Game Studio/GS_Engine/vendor/GLAD/glad.h"
#include "W:/Game Studio/GS_Engine/vendor/GLFW/glfw3.h"

#include "Window.h"

#include "VBO.h"
#include "IBO.h"
#include "VAO.h"
#include "Program.h"

GS_CLASS Renderer : ESystem
{
public:
	Renderer(Window * WD);
	~Renderer();

	void OnUpdate(float DeltaTime);
	void Draw(VBO * vbo, IBO * ibo, VAO * vao, Program * progr) const;

private:
	unsigned int DrawCalls;

	Window * WindowInstanceRef;
};

