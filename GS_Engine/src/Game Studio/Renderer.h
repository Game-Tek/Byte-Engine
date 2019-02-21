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
#include "Scene.h"

GS_CLASS Renderer : public ESystem
{
public:
	Renderer(Window * WD);
	~Renderer();

	void OnUpdate() override;
	void Draw(IBO* ibo, VAO* vao, Program* progr) const;

	Scene & GetScene() { return ActiveScene; }

protected:
	Scene ActiveScene;

private:
	uint32 DrawCalls = 0;

	Window * WindowInstanceRef;

	Matrix4 ProjectionMatrix;

	//Returns a symetric perspective frustrum.
	static Matrix4 BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far)
	{
		const float Tangent = GSM::Tan(FOV / 2.0f); //Tangent of half the vertical view angle.
		const float Height = Near * Tangent;		//Half height of the near plane(point that says where it is placed).
		const float Width = Height * AspectRatio;	//Half width of the near plane(point that says where it is placed).

		return BuildPerspectiveFrustrum(-Width, Width, -Height, Height, Near, Far);
	}

	//Returns a perspective frustrum.
	static Matrix4 BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far)
	{
		Matrix4 Result;
		Result.Identity();

		Result[0] = 2.0f * Near / (Right - Left);
		Result[5] = 2.0f * Near / (Top - Bottom);
		Result[8] = (Right + Left) / (Right - Left);
		Result[9] = (Top + Bottom) / (Top - Bottom);
		Result[10] = -(Far + Near) / (Far - Near);
		Result[11] = -1.0f;
		Result[14] = -(2.0f * Far * Near) / (Far - Near);
		Result[15] = 0.0f;

		return Result;
	}
};

