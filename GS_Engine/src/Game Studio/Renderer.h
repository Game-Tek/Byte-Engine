#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "Window.h"

#include "Scene.h"

class IBO;
class VAO;
class Program;

class GBufferRenderPass;
class LightRenderPass;

GS_CLASS Renderer : public ESystem
{
public:
	Renderer(Window * WD);
	virtual ~Renderer();

	void OnUpdate() override;

	Scene * GetScene() { return &ActiveScene; }

	GBufferRenderPass * GetGBufferPass() const { return GBufferPass; }

protected:
	//Renders a whole frame.
	void RenderFrame() const;

	Scene ActiveScene;

	GBufferRenderPass * GBufferPass;
	LightRenderPass * LightingRenderPass;

private:
	uint32 DrawCalls = 0;

	Window * WindowInstanceRef;
};

