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

class TestObject : public WorldObject
{
	StaticMesh MyStaticMesh;
	Texture MyTexture;
	StaticMeshRenderComponent* MeshRender = nullptr;
	Material* MyMaterial = nullptr;
	Camera MyCamera;

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

		auto a = Matrix4(Rotator(40, 0, 0));
		auto b = Quaternion(Rotator(40, 0, 0));
		auto c = Vector3(Rotator(32, 0, 0));
		auto d = Rotator(c);
	}

	~TestObject()
	{
	}

	void OnUpdate() override
	{
		auto i_m = GS::Application::Get()->GetInputManager();
		
		auto rot = Rotator(i_m.GetMouseOffset().Y * 0.5, i_m.GetMouseOffset().X * 0.5, 0);
		
		Vector3 pos;
		pos.X += i_m.GetKeyState(KeyboardKeys::D) ? 0.5 : 0;
		pos.X -= i_m.GetKeyState(KeyboardKeys::A) ? 0.5 : 0;
		pos.Y += i_m.GetKeyState(KeyboardKeys::SpaceBar) ? 0.5 : 0;
		pos.Y -= i_m.GetKeyState(KeyboardKeys::LShift) ? 0.5 : 0;
		pos.Z += i_m.GetKeyState(KeyboardKeys::W) ? 0.5 : 0;
		pos.Z -= i_m.GetKeyState(KeyboardKeys::S) ? 0.5 : 0;
		
		MyCamera.GetTransform().Position += Matrix4(rot) * pos;


		//MyCamera.GetTransform().Rotation = GSM::RotatorToQuaternion(rot) * MyCamera.GetTransform().Rotation;
		//MyCamera.GetTransform().Rotation *= corrected;
		//MyCamera.GetTransform().Rotation *= Quaternion(0, 1, 0, 0);

		MyCamera.GetFOV() -= i_m.GetMouseState().MouseWheelMove;
	}

	[[nodiscard]] const char* GetName() const override { return "TestObject"; }
};
