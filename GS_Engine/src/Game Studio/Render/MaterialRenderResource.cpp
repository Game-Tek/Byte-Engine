#include "MaterialRenderResource.h"

#include "Renderer.h"

MaterialRenderResource::MaterialRenderResource(const MaterialRenderResourceCreateInfo& MRRCI_) : RenderResource(),
                                                                                                 referenceMaterial(MRRCI_.ParentMaterial),
                                                                                                 textures(MRRCI_.textures),
bindingsIndex(MRRCI_.BindingsIndex)
{
}
