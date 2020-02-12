#pragma once

#include "Object.h"

class Renderer;
class FString;

namespace RAPI
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
        RAPI::CommandBuffer* CommandBuffer = nullptr;

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
     * \param name Reference to an FString in which the name will be stored.
     */
    virtual void GetRenderableTypeName(FString& name) = 0;

    virtual uint32 RegisterComponent(Renderer* renderer, class RenderComponent* renderComponent);
};