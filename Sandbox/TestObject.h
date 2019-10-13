#pragma once

#include <Game Studio/Game/WorldObject.h>
#include <Game Studio/Render/RenderComponent.h>

#include <Game Studio/Game/World.h>
#include <Game Studio/Render/Scene.h>
#include "Render/StaticMeshRenderComponent.h"

class TestObject : public WorldObject
{
	StaticMesh MyStaticMesh;
	StaticMeshRenderComponent* MeshRender = nullptr;

public:
	TestObject() : MyStaticMesh(FString("W:/Game Studio/bin/Sandbox/Debug-x64/Sphere.obj"))
	{
		MeshRender = GetWorld()->GetScene().CreateRenderComponent<StaticMeshRenderComponent>(this);
		MeshRender->SetStaticMesh(&MyStaticMesh);
	}

	~TestObject()
	{
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
