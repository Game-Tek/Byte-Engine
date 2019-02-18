#include "Camera.h"
#include "GSM.hpp"

Camera::Camera(const float FOV) : FOV(FOV)
{
}

void Camera::SetFocusDistance(const Vector3 & Object)
{
	FocusDistance = GSM::VectorLength(this->GetPosition() - Object);
	
	return;
}