#pragma once

#include "RAPI/GraphicsPipeline.h"

class RenderGroupBase
{
	/**
     * \brief Defines how many instances can be rendered every instanced draw call.
     * Also used to switch binded buffers when rendering.
     * Usually determined by the per instance data size and the max buffer size.\n
     * max buffer size/per instance data size = maxInstanceCount.
     */
    uint32 maxInstanceCount = 0;
};

class BindingsType
{
    RAPI::BindingsPool* bindingsPool = nullptr;
    RAPI::BindingsSet* bindingsSet = nullptr;

public:
};

/**
 * \brief A render group represents a group of meshes that share the same pipeline, hence can be rendered together.
 */
class RenderGroup
{
    RAPI::Pipeline* pipeline = nullptr;
    FVector<RAPI::RenderMesh*> meshes;
public:
};

class RenderGroupManager
{
	
public:
};