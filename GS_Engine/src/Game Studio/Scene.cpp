#include "Scene.h"

Scene::Scene() : ObjectList(50), ViewMatrix(), ProjectionMatrix(BuildPerspectiveMatrix(GSM::DegreesToRadians(45.0f), 1280.0f / 720.0f, 0.01f, 100.0f))
{
}

void Scene::AddWorldObject(WorldObject * Object)
{
	ObjectList.push_back(Object);

	return;
}

void Scene::RemoveWorldObject(WorldObject * Object)
{
	ObjectList.eraseObject(Object);

	return;
}

void Scene::UpdateViewMatrix()
{
	ViewMatrix[12] = GetCamera()->GetPosition().X;
	ViewMatrix[13] = GetCamera()->GetPosition().Y;
	ViewMatrix[14] = GetCamera()->GetPosition().Z;

	return;
}

//Returns a symetric perspective frustrum.
Matrix4 Scene::BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far)
{
	const float Tangent = GSM::Tan(FOV / 2.0f); //Tangent of half the vertical view angle.
	const float Height = Near * Tangent;		//Half height of the near plane(point that says where it is placed).
	const float Width = Height * AspectRatio;	//Half width of the near plane(point that says where it is placed).

	return BuildPerspectiveFrustrum(-Width, Width, -Height, Height, Near, Far);
}

//Returns a perspective frustrum.
Matrix4 Scene::BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far)
{
	Matrix4 Result;
	Result.Identity();

	Result[0] = 2.0f * Near / (Right - Left);
	Result[5] = 2.0f * Near / (Top - Bottom);
	Result[8] = (Right + Left) / (Right - Left);
	Result[9] = (Top + Bottom) / (Top - Bottom);
	Result[10] = -(Far + Near) / (Far - Near);
	Result[11] = -1.0f;
	Result[14] = -(2.0f * Far * Near) / (Far - Near);
	Result[15] = 0.0f;

	return Result;
}
