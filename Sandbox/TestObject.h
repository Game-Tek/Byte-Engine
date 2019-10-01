#pragma once

#include <Game Studio/Game/WorldObject.h>
#include <Game Studio/Render/RenderComponent.h>

#include <Game Studio/Game/World.h>
#include <Game Studio/Render/Scene.h>
#include <Game Studio/Resources/StaticMeshResourceManager.h>

class TestObject : public WorldObject
{
	StaticMeshRenderComponent* MeshRender = nullptr;
	StaticMesh* MyStaticMesh = nullptr;

public:
	TestObject() :  MeshRender(GetWorld()->GetScene().CreateStaticMeshRenderComponent(this)),
					MyStaticMesh(StaticMeshResourceManager::Get().GetResource(FString("W:/Game Studio/bin/Sandbox/Debug-x64/Sphere.obj")))
	{
		MeshRender->SetStaticMesh(MyStaticMesh);
	}

	~TestObject()
	{
		StaticMeshResourceManager::Get().ReleaseResource(MyStaticMesh);
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
