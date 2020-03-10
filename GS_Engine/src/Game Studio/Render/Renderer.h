#pragma once

#include "Core.h"
#include "Camera.h"
#include "Containers/FVector.hpp"
#include "Math/Matrix4.h"
#include "Game/SubWorlds.h"
#include "RenderComponent.h"

#include "RAPI/Window.h"
#include "RAPI/Framebuffer.h"
#include "RAPI/GraphicsPipeline.h"
#include "RAPI/UniformBuffer.h"
#include "RAPI/RenderContext.h"
#include "RAPI/RenderPass.h"

#include <map>
#include "Containers/Id.h"
#include "RAPI/Bindings.h"
#include "Game/StaticMesh.h"
#include "RenderableTypeManager.h"
#include "BindingsGroup.h"

class Material;
class StaticMeshResource;
class RenderProxy;
class PointLightRenderProxy;

//Stores all the data necessary for the RAPI to work. It's the RenderAPI representation of the game world.
class Renderer : public SubWorld
{
public:
	Renderer();
	virtual ~Renderer();

	[[nodiscard]] const char* GetName() const override { return "Scene"; }

	void OnUpdate() override;

	//Returns a pointer to the active camera.
	[[nodiscard]] Camera* GetActiveCamera() const { return ActiveCamera; }

	//Sets the active camera as the NewCamera.
	void SetCamera(Camera* NewCamera) const { ActiveCamera = NewCamera; }

	template <class T>
	T* CreateRenderComponent(RenderComponentCreateInfo* _RCCI)
	{
		RenderComponent* NRC = new T();
		NRC->SetOwner(_RCCI->Owner);
		this->RegisterRenderComponent(NRC, _RCCI);
		return static_cast<T*>(NRC);
	}

	void DrawMeshes(const RAPI::CommandBuffer::DrawIndexedInfo& _DrawInfo, RAPI::RenderMesh* Mesh_);
	void BindPipeline(RAPI::GraphicsPipeline* _Pipeline);

	RAPI::RenderMesh* CreateMesh(StaticMesh* _SM);

protected:
	//Used to count the amount of draw calls in a frame.
	GS_DEBUG_ONLY(uint64 DrawCalls = 0)
	GS_DEBUG_ONLY(uint64 InstanceDraws = 0)
	GS_DEBUG_ONLY(uint64 PipelineSwitches = 0)
	GS_DEBUG_ONLY(uint64 DrawnComponents = 0)

	friend RenderableTypeManager;
	
	FVector<RenderableTypeManager*> renderableTypeManagers;
	
	/* ---- RAPI Resources ---- */
	std::map<Id::HashType, RAPI::GraphicsPipeline*> Pipelines;
	FVector<class MaterialRenderResource*> materialRenderResources;
	std::map<StaticMesh*, RAPI::RenderMesh*> Meshes;
	std::map<GS_HASH_TYPE, RenderComponent*> ComponentToInstructionsMap;
	FVector<Pair<RAPI::BindingsPool*, RAPI::BindingsSet*>> bindings;

	RAPI::GraphicsPipeline* CreatePipelineFromMaterial(Material* _Mat) const;

	//Pointer to the active camera.
	mutable Camera* ActiveCamera = nullptr;

	//Render elements
	RAPI::Window* Win = nullptr;
	FVector<RAPI::Framebuffer*> Framebuffers;

	RAPI::RenderTarget* depthTexture = nullptr;

	RAPI::RenderDevice* renderDevice = nullptr;
	
	RAPI::Queue* graphicsQueue = nullptr;
	RAPI::Queue* transferQueue = nullptr;

	RAPI::RenderContext* RC = nullptr;
	RAPI::CommandBuffer* CB = nullptr;
	RAPI::RenderPass* RP = nullptr;
	RAPI::UniformBuffer* UB = nullptr;
	RAPI::BindingLayout* UL = nullptr;

	RAPI::RenderMesh* FullScreenQuad = nullptr;
	RAPI::GraphicsPipeline* FullScreenRenderingPipeline = nullptr;

	struct InstanceData
	{
	};

	struct MaterialData
	{
		uint32 textureIndices[8];
	};

	struct ViewData
	{
		GS_ALIGN(16) Matrix4 ViewMatrix;
		GS_ALIGN(16) Matrix4 ProjectionMatrix;
		GS_ALIGN(16) Matrix4 ViewProjectionMatrix;
	};

	FVector<ViewData> perViewData;
	FVector<InstanceData> perInstanceData;
	FVector<Matrix4> perInstanceTransform;
	FVector<MaterialData> perMaterialInstanceData;


	void UpdateViews();

	void RegisterRenderComponent(RenderComponent* _RC, RenderComponentCreateInfo* _RCCI);

	void UpdateRenderables();
	void RenderRenderables();
	
	//Returns a symmetric perspective frustum.
	static void BuildPerspectiveMatrix(Matrix4& matrix, float fov, float aspectRatio, float near,
	                                   float far);

	static void MakeOrthoMatrix(Matrix4& matrix, float right, float left, float top,
	                            float bottom, float near, float far);
};
