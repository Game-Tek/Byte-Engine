#pragma once

#include "Core.h"

#include "Scene.h"

class GS_API Renderer : public Object
{
	Scene* ActiveScene = nullptr;
public:


	void SetActiveScene(Scene* _NewScene) { ActiveScene = _NewScene; }

	[[nodiscard]] const char* GetName() const override { return "Renderer"; }
};