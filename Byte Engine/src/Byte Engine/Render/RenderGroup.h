#pragma once

#include "GAL/GraphicsPipeline.h"

/**
 * \brief A render group represents a group of meshes that share the same pipeline, hence can be rendered together.
 * A render group can consume a binding group so as to use the bindings it contains.
 */
class RenderGroup
{
    GAL::Pipeline* pipeline = nullptr;

public:
    struct RenderGroupCreateInfo
    {
        GAL::Pipeline* Pipeline = nullptr;
    };

    explicit RenderGroup(const RenderGroupCreateInfo& renderGroupCreateInfo);
};