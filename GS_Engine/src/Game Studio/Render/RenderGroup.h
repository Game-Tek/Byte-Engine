#pragma once

#include "RAPI/GraphicsPipeline.h"

/**
 * \brief A render group represents a group of meshes that share the same pipeline, hence can be rendered together.
 * A render group can consume a binding group so as to use the bindings it contains.
 */
class RenderGroup
{
    RAPI::Pipeline* pipeline = nullptr;

public:
    struct RenderGroupCreateInfo
    {
        RAPI::Pipeline* Pipeline = nullptr;
    };

    explicit RenderGroup(const RenderGroupCreateInfo& renderGroupCreateInfo);
};