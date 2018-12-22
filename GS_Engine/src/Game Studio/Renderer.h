#pragma once

#include "Core.h"

#include "EngineSystem.h"

GS_CLASS Renderer : public ESystem
{
public:
	Renderer();
	~Renderer();

	void Update(float DeltaTime);
};

