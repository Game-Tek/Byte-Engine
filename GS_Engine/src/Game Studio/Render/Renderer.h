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
#include "RenderableInstructions.h"
#include "Containers/Id.h"
#include "RAPI/Bindings.h"

class StaticMeshResource;
class RenderProxy;
class PointLightRenderProxy;

//Stores all the data necessary for the RAPI to work. It's the RAPIs representation of the game world.
class GS_API Renderer : public SubWorld
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


	void DrawMesh(const DrawInfo& _DrawInfo, class MeshRenderResource* Mesh_);
	void BindPipeline(GraphicsPipeline* _Pipeline);


	class MeshRenderResource* CreateMesh(StaticMesh* _SM);
	class MaterialRenderResource* CreateMaterial(Material* Material_);
protected:
	//Used to count the amount of draw calls in a frame.
	GS_DEBUG_ONLY(uint32 DrawCalls = 0)
	GS_DEBUG_ONLY(uint32 InstanceDraws = 0)
	GS_DEBUG_ONLY(uint32 PipelineSwitches = 0)
	GS_DEBUG_ONLY(uint32 DrawnComponents = 0)

	/* ---- RAPI Resources ---- */
	std::map<Id::HashType, GraphicsPipeline*> Pipelines;
	FVector<class MaterialRenderResource*> materialRenderResources;
	std::map<StaticMesh*, MeshRenderResource*> Meshes;
	std::map<GS_HASH_TYPE, RenderComponent*> ComponentToInstructionsMap;
	std::map<GS_HASH_TYPE, Pair<BindingsPool*, BindingsSet*>> bindings;

	GraphicsPipeline* CreatePipelineFromMaterial(Material* _Mat) const;

	//Pointer to the active camera.
	mutable Camera* ActiveCamera = nullptr;

	//Render elements
	Window* Win = nullptr;
	FVector<Framebuffer*> Framebuffers;

	Image* depthTexture = nullptr;

	RenderContext* RC = nullptr;
	RenderPass* RP = nullptr;
	UniformBuffer* UB = nullptr;
	BindingLayout* UL = nullptr;

	RenderMesh* FullScreenQuad = nullptr;
	GraphicsPipeline* FullScreenRenderingPipeline = nullptr;

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
	static void BuildPerspectiveMatrix(Matrix4& _Matrix, const float _FOV, const float _AspectRatio, const float _Near,
	                                   const float _Far);

	static void MakeOrthoMatrix(Matrix4& _Matrix, const float _Right, const float _Left, const float _Top,
	                            const float _Bottom, const float _Near, const float _Far);
};
