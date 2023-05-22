#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/System.hpp"
#include <GTSL/Range.hpp>
#include <GTSL/Math/Matrix.hpp>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Quaternion.h>
#include <GTSL/Vector.hpp>

class CameraSystem : public BE::System
{
public:
	MAKE_HANDLE(GTSL::uint32, Camera);

	CameraHandle AddCamera(const GTSL::Vector3& position)
	{
		m_rotationMatrices.EmplaceBack();
		m_fovs.EmplaceBack(45.0f);
		auto index = m_positionMatrices.GetLength();
		m_positionMatrices.EmplaceBack(position);
		return CameraHandle{ index };
	}

	void RemoveCamera(const CameraHandle& reference)
	{
		m_positionMatrices.Pop(reference());
		m_rotationMatrices.Pop(reference());
		m_fovs.Pop(reference());
	}

	void SetCameraRotation(cosnt CameraHandle& ref, const GTSL::Quaterion quaterion)
	{
		m_rotationMatrices[ref()] = { quaterion };
	}

	void SetCameraRotation(cosnt CameraHandle& ref, const GTSL::Matrix4 matrix4)
	{
		m_rotationMatrices[ref()] = matrix4;
	}

	GTSL::Matrix4 GetCameraTransform() const
	{
		return m_rotationMatrices[0] * m_positionMatrices[0];
	}

	void SetCameraPosition(const CameraHandle& reference, const GTSL::Vector3& pos)
	{
		GTSL::Math::SetTranslation(m_positionMatrices[reference()], pos);
	}

	void AddCameraPosition(const CameraHandle& reference, GTSL::Vector3& pos)
	{
		GTSL::Math::Translate(m_positionMatrices[reference()], pos);
	}

	void AddCameraRotation(const CameraHandle& reference, const GTSL::Quaternion& quaternion)
	{
		m_rotationMatrices[reference()] *= GTSL::Matrix4(quaternion);
	}

	void AddCameraRotation(const CameraHandle& reference, const GTSL::Matrix4& matrix4)
	{
		m_rotationMatrices[reference()] *= matrix4;
	}

	[[nodiscard]] GTSL::Range<const GTSL::float32*> GetFieldOfViews() const { return m_fovs; }
private:
	GTSL::Vector<GTSL::Matrix4, BE::PAR> m_positionMatrices;
	GTSL::Vector<GTSL::Matrix4, BE::PAR> m_rotationMatrices;
	GTSL::Vector<GTSL::float32, BE::PAR> m_fovs;
};