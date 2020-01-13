#pragma once

#include <Game Studio/Game/WorldObject.h>
#include <Game Studio/Render/RenderComponent.h>

#include <Game Studio/Game/World.h>
#include <Game Studio/Render/Renderer.h>
#include <Game Studio/Camera.h>
#include <Game Studio/Game/Texture.h>
#include <Game Studio/Render/Material.h>
#include "Render/StaticMeshRenderComponent.h"

class TestObject : public WorldObject
{
	StaticMesh MyStaticMesh;
	Texture MyTexture;
	StaticMeshRenderComponent* MeshRender = nullptr;
	Material* MyMaterial = nullptr;
	Camera MyCamera;

public:
	TestObject() : MyStaticMesh("Box"), MyTexture("Logo_Game-Tek")
	{
		MyMaterial = new Material("M_Base");
		MyStaticMesh.SetMaterial(MyMaterial);

		StaticMeshRenderComponentCreateInfo SMRCCI;
		SMRCCI.StaticMesh = &MyStaticMesh;
		SMRCCI.Owner = this;
 		MeshRender = GetWorld()->GetScene().CreateRenderComponent<StaticMeshRenderComponent>(&SMRCCI);

		MyCamera.SetPosition(Vector3(0, 50, -250));
		GetWorld()->GetScene().SetCamera(&MyCamera);
	}

	~TestObject()
	{
	}

	void OnUpdate() override
	{
		Vector3 pos = MyCamera.GetPosition();
		pos.X += GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::D) ? 0.5 : 0;
		pos.X -= GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::A) ? 0.5 : 0;
		pos.Y += GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::SpaceBar) ? 0.5 : 0;
		pos.Y -= GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::LShift) ? 0.5 : 0;
		pos.Z += GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::W) ? 0.5 : 0;
		pos.Z -= GS::Application::Get()->GetInputManager().GetKeyState(KeyboardKeys::S) ? 0.5 : 0;
		
		MyCamera.SetPosition(pos);
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
