#pragma once

#include <Game Studio/Game/WorldObject.h>
#include <Game Studio/Render/RenderComponent.h>

#include <Game Studio/Game/World.h>
#include <Game Studio/Render/Scene.h>
#include <Game Studio/Camera.h>
#include <Game Studio/Game/Texture.h>
#include "Render/StaticMeshRenderComponent.h"

#include "BaseMaterial.h"

class TestObject : public WorldObject
{
	StaticMesh MyStaticMesh;
	Texture MyTexture;
	StaticMeshRenderComponent* MeshRender = nullptr;
	BaseMaterial* Material = nullptr;
	Camera MyCamera;

public:
	TestObject() : MyStaticMesh("Box"), MyTexture("Logo_Game-Tek")
	{
		Material = new BaseMaterial(FString("M_Base"));
		MyStaticMesh.SetMaterial(Material);

		StaticMeshRenderComponentCreateInfo SMRCCI;
		SMRCCI.StaticMesh = &MyStaticMesh;
		SMRCCI.Owner = this;
 		MeshRender = GetWorld()->GetScene().CreateRenderComponent<StaticMeshRenderComponent>(&SMRCCI);

		GetWorld()->GetScene().SetCamera(&MyCamera);
	}

	~TestObject()
	{
	}

	void OnUpdate() override
	{
		Vector3 pos = MyCamera.GetPosition();
		pos.X += GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::D) ? 0.1 : 0;
		pos.X -= GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::A) ? 0.1 : 0;
		pos.Y += GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::SpaceBar) ? 0.1 : 0;
		pos.Y -= GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::LShift) ? 0.1 : 0;
		pos.Z += GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::W) ? 0.1 : 0;
		pos.Z -= GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::S) ? 0.1 : 0;

		MyCamera.SetPosition(pos);
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
