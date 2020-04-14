#pragma once

#include "Core.h"
#include "Camera.h"
#include <GTSL/Vector.hpp>
#include "Game/SubWorlds.h"
#include "RenderComponent.h"

#include "GAL/Window.h"
#include "GAL/Framebuffer.h"
#include "GAL/GraphicsPipeline.h"
#include "GAL/UniformBuffer.h"
#include "GAL/RenderContext.h"
#include "GAL/RenderPass.h"

#include <map>
#include <GTSL/Id.h>
#include "GAL/Bindings.h"
#include "Game/StaticMesh.h"
#include "RenderableTypeManager.h"
#include "BindingsGroup.h"

class Material;
class StaticMeshResource;
class RenderProxy;
class PointLightRenderProxy;

//Stores all the data necessary for the GAL to work. It's the RenderAPI representation of the game world.
class Renderer : public SubWorld
{
public:
	Renderer();
	virtual ~Renderer();

	[[nodiscard]] const char* GetName() const override { return "Scene"; }

	//Returns a pointer to the active camera.
	[[nodiscard]] Camera* GetActiveCamera() const { return ActiveCamera; }

	//Sets the active camera as the NewCamera.
	void SetCamera(Camera* NewCamera) const { ActiveCamera = NewCamera; }

	void DrawMeshes(const GAL::CommandBuffer::DrawIndexedInfo& _DrawInfo, GAL::RenderMesh* Mesh_);
	void BindPipeline(GAL::GraphicsPipeline* _Pipeline);

	GAL::RenderMesh* CreateMesh(StaticMesh* _SM);

protected:
	//Used to count the amount of draw calls in a frame.
	BE_DEBUG_ONLY(uint64 DrawCalls = 0)
	BE_DEBUG_ONLY(uint64 InstanceDraws = 0)
	BE_DEBUG_ONLY(uint64 PipelineSwitches = 0)
	BE_DEBUG_ONLY(uint64 DrawnComponents = 0)

	friend RenderableTypeManager;
	
	GTSL::Vector<RenderableTypeManager*> renderableTypeManagers;
	
	/* ---- GAL Resources ---- */
	std::map<GTSL::Id64::HashType, GAL::GraphicsPipeline*> Pipelines;
	GTSL::Vector<class MaterialRenderResource*> materialRenderResources;
	std::map<StaticMesh*, GAL::RenderMesh*> Meshes;
	std::map<uint64, RenderComponent*> ComponentToInstructionsMap;
	GTSL::Vector<Pair<GAL::BindingsPool*, GAL::BindingsSet*>> bindings;

	GAL::GraphicsPipeline* CreatePipelineFromMaterial(Material* _Mat) const;

	//Pointer to the active camera.
	mutable Camera* ActiveCamera = nullptr;

	//Render elements
	GAL::RenderDevice* renderDevice = nullptr;

	GAL::Queue* graphicsQueue = nullptr;
	GAL::Queue* transferQueue = nullptr;

	GAL::Window* Win = nullptr;
	GTSL::Vector<GAL::Framebuffer*> Framebuffers;

	GAL::RenderTarget* depthTexture = nullptr;
	
	GAL::RenderContext* RC = nullptr;
	GAL::CommandBuffer* graphicsCommandBuffer = nullptr;
	GAL::CommandBuffer* transferCommandBuffer = nullptr;
	GAL::RenderPass* RP = nullptr;

	GAL::RenderMesh* FullScreenQuad = nullptr;
	GAL::GraphicsPipeline* FullScreenRenderingPipeline = nullptr;

	struct InstanceData
	{
	};

	struct MaterialData
	{
		uint32 textureIndices[8];
	};

	GTSL::Vector<InstanceData> perInstanceData;
	GTSL::Vector<GTM::Matrix4> perInstanceTransform;
	GTSL::Vector<MaterialData> perMaterialInstanceData;


	void UpdateViews();

	void UpdateRenderables();
	void RenderRenderables();
};
