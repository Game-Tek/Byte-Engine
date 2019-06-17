#include "Renderer.h"

#include <GLAD/glad.h>
#include <GLFW/glfw3.h>

#include "Application.h"

#include "GSM.hpp"

Renderer::Renderer(Window * WD)
{
}

Renderer::~Renderer()
{
}

void Renderer::RenderFrame() const
{
}

void Renderer::OnUpdate()
{
	ActiveScene.OnUpdate();

	RenderFrame();

	return;
}