#pragma once

#include "Core.h"
#include "Camera.h"
#include "Containers/FVector.hpp"
#include "Math/Matrix4.h"
#include "Game/SubWorlds.h"
#include "RenderComponent.h"

#include "ScreenQuad.h"
#include "RAPI/Window.h"
#include "RAPI/Framebuffer.h"
#include "RAPI/GraphicsPipeline.h"
#include "RAPI/UniformBuffer.h"
#include "RAPI/RenderContext.h"
#include "RAPI/RenderPass.h"

#include "StaticMeshRenderComponent.h"
#include "RenderResourcesManager.h"
#include <map>
#include "RenderableInstructions.h"

class StaticMeshResource;
class RenderProxy;
class PointLightRenderProxy;

//Stores all the data necessary for the RAPI to work. It's the RAPIs representation of the game world.
class GS_API Scene : public SubWorld
{
public:
	Scene();
	virtual ~Scene();

	void OnUpdate() override;	

	//Returns a pointer to the active camera.
	[[nodiscard]] Camera* GetActiveCamera() const { return ActiveCamera; }
	[[nodiscard]] const Matrix4& GetViewMatrix() const { return ViewMatrix; }
	[[nodiscard]] const Matrix4& GetProjectionMatrix() const { return ProjectionMatrix; }
	[[nodiscard]] const Matrix4& GetVPMatrix() const { return ViewProjectionMatrix; }

	//Sets the active camera as the NewCamera.
	void SetCamera(Camera * NewCamera) { ActiveCamera = NewCamera; }

	template<class T>
	T* CreateRenderComponent(WorldObject* _Owner) const
	{
		RenderComponent* NRC = new T();
		RenderableInstructionsMap.try_emplace(Id(NRC->GetRenderableTypeName()).GetID(), NRC->GetRenderableInstructions());
		RenderComponents.emplace_back(NRC);
		return static_cast<T*>(NRC);
	}

	[[nodiscard]] const char* GetName() const override { return "Scene"; }

	RenderResourcesManager ResourcesManager;

	void DrawMesh(const DrawInfo& _DI);
protected:

	uint32 DrawCalls = 0;

	mutable std::map<Id::HashType, RenderableInstructions> RenderableInstructionsMap;

	//Scene elements
	mutable FVector<RenderComponent*> RenderComponents;

	//Pointer to the active camera.
	Camera* ActiveCamera = nullptr;

	//Render elements
	Window* Win = nullptr;
	FVector<Framebuffer*> Framebuffers;
	ScreenQuad MyQuad = {};
	RenderContext* RC = nullptr;
	RenderPass* RP = nullptr;
	GraphicsPipeline* GP = nullptr;
	UniformBuffer* UB = nullptr;
	UniformLayout* UL = nullptr;

	//Matrix necessary to represent the active camera's view position.
	Matrix4 ViewMatrix;
	//Matrix necessary to represent the active camera's view angle.
	Matrix4 ProjectionMatrix;
	//Matrix to represent the multiplication of the view and projection matrix.
	Matrix4 ViewProjectionMatrix;

	void UpdateMatrices();

	//Returns a symetric perspective frustrum.
	static Matrix4 BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far);

	//Returns a perspective frustrum.
	static Matrix4 BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far);
};