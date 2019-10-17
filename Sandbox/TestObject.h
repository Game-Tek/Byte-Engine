#pragma once

#include <Game Studio/Game/WorldObject.h>
#include <Game Studio/Render/RenderComponent.h>

#include <Game Studio/Game/World.h>
#include <Game Studio/Render/Scene.h>
#include <Game Studio/Camera.h>
#include "Render/StaticMeshRenderComponent.h"

#include "BaseMaterial.h"

class TestObject : public WorldObject
{
	StaticMesh MyStaticMesh;
	StaticMeshRenderComponent* MeshRender = nullptr;
	BaseMaterial* Material = nullptr;
	Camera MyCamera;

public:
	TestObject() : MyStaticMesh(FString("Sphere"))
	{
		Material = new BaseMaterial(FString("M_Base"));
		MyStaticMesh.SetMaterial(Material);

 		MeshRender = GetWorld()->GetScene().CreateRenderComponent<StaticMeshRenderComponent>(this);
		MeshRender->SetStaticMesh(&MyStaticMesh);

		GetWorld()->GetScene().SetCamera(&MyCamera);
	}

	~TestObject()
	{
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
