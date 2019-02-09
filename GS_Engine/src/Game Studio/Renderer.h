#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "W:/Game Studio/GS_Engine/vendor/GLAD/glad.h"
#include "W:/Game Studio/GS_Engine/vendor/GLFW/glfw3.h"

#include "Window.h"

#include "Matrix4.h"

#include "VBO.h"
#include "IBO.h"
#include "VAO.h"
#include "Program.h"

GS_CLASS Renderer : public ESystem
{
public:
	Renderer(Window * WD);
	~Renderer();

	void OnUpdate() override;
	void Draw(VBO * vbo, IBO * ibo, VAO * vao, Program * progr) const;

private:
	unsigned int DrawCalls = 0;

	Window * WindowInstanceRef;

	Matrix4 ProjectionMatrix;

	//Builds 4x4 matrix to create a projection matrix. FOV needs to be in radians. 
	Matrix4 BuildProjectionMatrix(float FOV, float Near, float Far, float Right, float Bottom, float Left, float Top)
	{
		return Matrix4(2 / (Right - Left), 0, 0, 0, 0, 2 / (Top - Bottom), 0, 0, 0, 0, -(Far + Near) / (Far - Near), -1, -Near * (Right + Left) / (Right - Left), -Near * (Top + Bottom) / (Top - Bottom), 2 * Far * Near / (Near - Far), 0);
	}
};

