#pragma once

#include "Core.h"

#include "Object.h"

namespace GTSL {
	class Id64;
}

class Renderer;

namespace GAL
{
    class CommandBuffer;
}

/**
 * \brief This class manages and renders an specific type of renderable object.
 * The main renderer holds a collection of children of this class so the
 * renderer stager can call to render the different types when appropriate.
 */
class RenderableTypeManager : public Object
{
public:
	struct RenderableTypeManagerCreateInfo
	{
        uint8 MaxFramesInFlight = 0;
	};
	
    RenderableTypeManager();
	~RenderableTypeManager();

    /**
     * \brief Holds all the information RenderableTypeManager::DrawObjects consumes.
     */
    struct DrawObjectsInfo
    {
	    /**
         * \brief Command buffer to submit all commands to.
         */
        GAL::CommandBuffer* CommandBuffer = nullptr;

	    /**
         * \brief Pointer to the active view projection matrix.
         */
        class Matrix4* ViewProjectionMatrix = nullptr;
    };
	
    /**
     * \brief This methods starts rendering of all the objects in this RenderableTypeManager.
     */
    virtual void DrawObjects(const DrawObjectsInfo& drawObjectsInfo) = 0;

    /**
     * \brief Returns the name of the RenderableType this instance of the class takes care of rendering.
     * \param name Reference to an GTSL::String in which the name will be stored.
     */
	[[nodiscard]] virtual GTSL::Id64 GetRenderableTypeName() const = 0;

    virtual uint32 RegisterComponent(Renderer* renderer, class RenderComponent* renderComponent);
};