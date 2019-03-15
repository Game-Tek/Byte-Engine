#pragma once

#include "Core.h"
#include "Camera.h"
#include "FVector.hpp"
#include "Matrix4.h"
#include "EngineSystem.h"

class StaticMesh;
class RenderProxy;

//Stores all the data necessary for the renderer to work. It's the renderers representation of the game world.
GS_CLASS Scene : public ESystem
{
public:
	Scene();
	virtual ~Scene() = default;

	virtual void OnUpdate() override;

	void AddObject(RenderProxy * Object);
	void RemoveObject(RenderProxy * Object);

	//Returns a pointer to the active camera.
	Camera * GetActiveCamera() const { return ActiveCamera; }
	const Matrix4 * GetViewMatrix() const { return &ViewMatrix; }
	const Matrix4 * GetProjectionMatrix() const { return &ProjectionMatrix; }
	const Matrix4 * GetVPMatrix() const { return &VPMatrix; }

	//Sets the active camera as the NewCamera.
	void SetCamera(Camera * NewCamera) { ActiveCamera = NewCamera; }

	FVector<RenderProxy *> RenderProxyList;

protected:
	//Pointer to the active camera.
	Camera * ActiveCamera = nullptr;

	//Matrix necessary to represent the active camera's view position.
	Matrix4 ViewMatrix;

	//Matrix necessary to represent the active camera's view angle.
	Matrix4 ProjectionMatrix;

	//Matrix to represent the multiplication of the view and projection matrix.
	Matrix4 VPMatrix;

	//Updates the view matrix to follow the active's camera position.
	void UpdateViewMatrix();

	//Updated the projection to keep up with window size changes and FOV changes.
	void UpdateProjectionMatrix();

	void UpdateVPMatrix();

	//Returns a symetric perspective frustrum.
	static Matrix4 BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far);

	//Returns a perspective frustrum.
	static Matrix4 BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far);
};

INLINE void Scene::UpdateVPMatrix()
{
	VPMatrix = ProjectionMatrix * ViewMatrix;

	return;
}