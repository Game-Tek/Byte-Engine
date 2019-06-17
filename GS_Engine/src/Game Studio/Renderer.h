#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "Window.h"

#include "Scene.h"

GS_CLASS Renderer : public ESystem
{
public:
	Renderer(Window * WD);
	virtual ~Renderer();

	void OnUpdate() override;

	const Scene & GetScene() const { return ActiveScene; }
protected:
	//Renders a whole frame.
	void RenderFrame() const;

	Scene ActiveScene;

private:
	uint32 DrawCalls = 0;
};

