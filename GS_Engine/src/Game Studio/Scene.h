#pragma once

#include "Core.h"
#include "Camera.h"
#include "FVector.hpp"
#include "Matrix4.h"
#include "StaticMesh.h"

class StaticMesh;

//Stores all the data necessary for the renderer to work. It's the renderers representation of the game world.
GS_CLASS Scene
{
public:
	Scene();
	virtual ~Scene() = default;

	void AddStaticMesh(StaticMesh * Object);
	void RemoveStaticMesh(StaticMesh * Object);

	//Returns a pointer to the active camera.
	Camera * GetCamera() const { return ActiveCamera; }

	//Sets the active camera as the NewCamera.
	void SetCamera(Camera * NewCamera) { ActiveCamera = NewCamera; }

	FVector<StaticMesh *> StaticMeshList;

protected:
	//Pointer to the active camera.
	Camera * ActiveCamera = nullptr;

	//Matrix necessary to represent the active camera's view position.
	Matrix4 ViewMatrix;

	//Matrix necessary to represent the active camera's view angle.
	Matrix4 ProjectionMatrix;

	//Updates the view matrix to follow the active's camera position.
	void UpdateViewMatrix();

	void UpdateProjectionMatrix();

	//Returns a symetric perspective frustrum.
	static Matrix4 BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far);

	//Returns a perspective frustrum.
	static Matrix4 BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far);
};

