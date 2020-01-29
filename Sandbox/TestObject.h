#pragma once

#include <Game Studio/Game/WorldObject.h>
#include <Game Studio/Render/RenderComponent.h>

#include <Game Studio/Game/World.h>
#include <Game Studio/Render/Renderer.h>
#include <Game Studio/Camera.h>
#include <Game Studio/Game/Texture.h>
#include <Game Studio/Render/Material.h>
#include "Render/StaticMeshRenderComponent.h"
#include "Math/GSM.hpp"
#include "Debug/Logger.h"

class TestObject : public WorldObject
{
	StaticMesh MyStaticMesh;
	Texture MyTexture;
	StaticMeshRenderComponent* MeshRender = nullptr;
	Material* MyMaterial = nullptr;
	Camera MyCamera;
	Rotator accumRotation;

public:
	TestObject() : MyStaticMesh("hydrant"), MyTexture("Logo_Game-Tek")
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
		auto i_m = GS::Application::Get()->GetInputManager();
		
		accumRotation += Rotator(i_m.GetMouseOffset().X * 50, (-i_m.GetMouseOffset().Y * 50) * GSM::Cosine(accumRotation.X), (-i_m.GetMouseOffset().Y * 50) * GSM::Sine(accumRotation.X));
		
		Vector3 pos;
		pos.X += i_m.GetKeyState(KeyboardKeys::D) ? 0.5 : 0;
		pos.X -= i_m.GetKeyState(KeyboardKeys::A) ? 0.5 : 0;
		pos.Y += i_m.GetKeyState(KeyboardKeys::SpaceBar) ? 0.5 : 0;
		pos.Y -= i_m.GetKeyState(KeyboardKeys::LShift) ? 0.5 : 0;
		pos.Z += i_m.GetKeyState(KeyboardKeys::W) ? 0.5 : 0;
		pos.Z -= i_m.GetKeyState(KeyboardKeys::S) ? 0.5 : 0;

		//pos *= MyCamera.GetTransform().Rotation;
		MyCamera.GetTransform().Position += pos;

		//MyCamera.GetTransform().Rotation = Quaternion(accumRotation);

		MyCamera.GetFOV() -= i_m.GetMouseState().MouseWheelMove;
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
