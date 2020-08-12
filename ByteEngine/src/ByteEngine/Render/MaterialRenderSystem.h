#pragma once

#include <GTSL/Vector.hpp>
#include "ByteEngine/Game/System.h"

class MaterialRenderSystem : public System
{
public:
	MaterialRenderSystem() : System("MaterialRenderSystem")
	{}

	void Initialize(const InitializeInfo& initializeInfo) override {}
	void Shutdown(const ShutdownInfo& shutdownInfo) override{}
	
	void CreateMaterial()
	{
	}

private:

};