#include "Scene.h"

#include "GSM.hpp"

Scene::Scene() : RenderProxyList(50)
{
}

void Scene::OnUpdate()
{
	//----UPDATE MATRICES----
	UpdateViewMatrix();
	UpdateProjectionMatrix();
	UpdateVPMatrix();
	//----UPDATE MATRICES----
}


void Scene::AddObject(RenderProxy * Object)
{
	RenderProxyList.push_back(Object);

	return;
}

void Scene::RemoveObject(RenderProxy * Object)
{
	RenderProxyList.eraseObject(Object);

	return;
}

void Scene::UpdateViewMatrix()
{
	//We get and store the camera's position so as to not access it several times.
	const Vector3 CamPos = GetActiveCamera()->GetPosition();

	//We set the view matrix's corresponding component to the inverse of the camera's position to make the matrix a translation matrix in the opposite direction of the camera.
	ViewMatrix[12] = -CamPos.X;
	ViewMatrix[13] = -CamPos.Y;
	ViewMatrix[14] = -CamPos.Z;

	return;
}

void Scene::UpdateProjectionMatrix()
{
	ProjectionMatrix = BuildPerspectiveMatrix(GSM::DegreesToRadians(45.0f), 1280.0f / 720.0f, 0.1f, 500.0f);

	return;
}

//Returns a symetric perspective frustrum.
Matrix4 Scene::BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far)
{
	const float Tangent = GSM::Tan(FOV * 0.5f); //Tangent of half the vertical view angle.
	const float Height = Near * Tangent;		//Half height of the near plane(point that says where it is placed).
	const float Width = Height * AspectRatio;	//Half width of the near plane(point that says where it is placed).

	return BuildPerspectiveFrustrum(Width, -Width, Height, -Height, Near, Far);
}

//Returns a perspective frustrum.
Matrix4 Scene::BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far)
{
	Matrix4 Result;

	Result[0] = (2.0f * Near) / (Right - Left);
	Result[5] = (2.0f * Near) / (Top - Bottom);
	Result[8] = (Right + Left) / (Right - Left);
	Result[9] = (Top + Bottom) / (Top - Bottom);
	Result[10] = -((Far + Near) / (Far - Near));
	Result[11] = -1.0f;
	Result[14] = -((2.0f * Far * Near) / (Far - Near));
	Result[15] = 0.0f;

	return Result;
}
