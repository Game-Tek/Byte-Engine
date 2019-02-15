#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "Window.h"

#include "Matrix4.h"

#include "VBO.h"
#include "IBO.h"
#include "VAO.h"
#include "Program.h"

#include "GSM.hpp"

#include <cmath>

GS_CLASS Renderer : public ESystem
{
public:
	Renderer(Window * WD);
	~Renderer();

	void OnUpdate() override;
	void Draw(IBO* ibo, VAO* vao, Program* progr) const;

private:
	uint32 DrawCalls = 0;

	Window * WindowInstanceRef;

	Matrix4 ProjectionMatrix;

	/*
	Matrix4 BuilProjectionMatrix(const float FOV, const float AspectRatio, const float Near, const float Far) const
	{
		const float Top = Near * tan(FOV * 2);
		const float Bottom = -Top;
		const float Right = Top * AspectRatio;
		const float Left = -Right;	

		return Matrix4(2.0f * Near / (Right - Left), 0.0f, 0.0f, 0.0f, 0.0f, 2.0f * Near / (Top - Bottom), 0.0f, 0.0f, (Right + Left) / (Right - Left), (Top + Bottom) / (Top - Bottom), -((Far + Near) / (Far - Near)), -1.0f, 0.0f, 0.0f, -((2.0f * Far * Near) / (Far - Near)), 0.0f);
	}
	*/

	Matrix4 BuildPerspectiveMatrix(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far)
	{
		Matrix4 Result;
		Result.Identity();

		Result[0] = (2.0f * Near) / (Right - Left);
		Result[5] = (2.0f * Near) / (Top - Bottom);
		Result[8] = (Right + Left) / (Right - Left);
		Result[9] = (Top + Bottom) / (Top - Bottom);
		Result[10] = (-(Far + Near) / (Far - Near));
		Result[11] = -1.0f;
		Result[14] = (-(2.0f * Far * Near) / (Far - Near));

		return Result;
	}
};

