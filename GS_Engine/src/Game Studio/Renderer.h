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

	Scene & GetScene() { return ActiveScene; }

protected:
	//Renders a whole frame.
	void RenderFrame(IBO * ibo, VAO * vao, Program * progr) const;

	Scene ActiveScene;

private:
	uint32 DrawCalls = 0;

	Window * WindowInstanceRef;
};

