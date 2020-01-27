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
		//
		MyCamera.GetTransform().Rotation *= Quaternion(Rotator(i_m.GetMouseOffset().Y, i_m.GetMouseOffset().X, 0));
		//
		auto totrot = Rotator(i_m.GetMouseState().MousePosition.Y, i_m.GetMouseState().MousePosition.X, 0);

		GS_LOG_MESSAGE("NormMousePos: X: %f, Y: %f", i_m.GetMouseState().MousePosition.X, i_m.GetMouseState().MousePosition.Y)
		
		Vector3 pos;
		pos.X += i_m.GetKeyState(KeyboardKeys::D) ? 0.5 : 0;
		pos.X -= i_m.GetKeyState(KeyboardKeys::A) ? 0.5 : 0;
		pos.Y += i_m.GetKeyState(KeyboardKeys::SpaceBar) ? 0.5 : 0;
		pos.Y -= i_m.GetKeyState(KeyboardKeys::LShift) ? 0.5 : 0;
		pos.Z += i_m.GetKeyState(KeyboardKeys::W) ? 0.5 : 0;
		pos.Z -= i_m.GetKeyState(KeyboardKeys::S) ? 0.5 : 0;

		pos *= MyCamera.GetTransform().Rotation;
		MyCamera.GetTransform().Position += pos;


		//MyCamera.GetTransform().Rotation = GSM::RotatorToQuaternion(rot) * MyCamera.GetTransform().Rotation;
		//MyCamera.GetTransform().Rotation *= corrected;
		//MyCamera.GetTransform().Rotation *= Quaternion(0, 1, 0, 0);

		//MyCamera.GetFOV() -= i_m.GetMouseState().MouseWheelMove;
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
