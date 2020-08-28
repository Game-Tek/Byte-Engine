#pragma once

#include "ByteEngine/Game/System.h"

#include "ByteEngine/Game/Tasks.h"

#include <GTSL/Array.hpp>

class MaterialSystem;
class RenderSystem;
/**
 * \brief A render group represents a group of meshes that share the same characteristics and can be rendered together.
 */
class RenderGroup : public System
{
public:
	virtual GTSL::Array<TaskDependency, 8> GetRenderDependencies() = 0;

	struct RenderInfo
	{
		GameInstance* GameInstance;
		RenderSystem* RenderSystem;
		MaterialSystem* MaterialSystem;
	};
	virtual void Render(const RenderInfo&) = 0;
};