#pragma once

#include <assimp/Importer.hpp>
#include <assimp/scene.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Math/Matrix4.h>
#include <GTSL/Math/Quaternion.h>

#include "ResourceManager.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Application/AllocatorReferences.h"

struct Bone
{
	GTSL::Matrix4 Offset;
	uint32 AffectedBone[4];
	float32 EffectIntensity[4];
};

struct Skeleton
{
	GTSL::Vector<Bone, BE::PAR> Bones;
	GTSL::FlatHashMap<uint32, BE::PAR> BonesMap;
};

struct Animation
{
	struct BoneAnimationData
	{
		GTSL::Vector3 Position; GTSL::Quaternion Rotation; GTSL::Vector3 Scale;
	};
	
	struct Frame
	{
		GTSL::Vector<BoneAnimationData, BE::PAR> Bones;
	};

	GTSL::Vector<Frame, BE::PAR> Frames;
	
	uint32 FrameCount;
	float32 FPS;
};

class AnimationResourceManager : ResourceManager
{
public:
	AnimationResourceManager()
	{

	}

private:
	static GTSL::Matrix4 assimpMatrixToMatrix(const aiMatrix4x4 assimpMatrix)
	{
		return GTSL::Matrix4(
			assimpMatrix.a1, assimpMatrix.a2, assimpMatrix.a3, assimpMatrix.a4,
			assimpMatrix.b1, assimpMatrix.b2, assimpMatrix.b3, assimpMatrix.b4,
			assimpMatrix.c1, assimpMatrix.c2, assimpMatrix.c3, assimpMatrix.c4,
			assimpMatrix.d1, assimpMatrix.d2, assimpMatrix.d3, assimpMatrix.d4
		);
	}

	static Id assimpStringToId(const aiString& aiString) {
		return Id(GTSL::Range<const utf8*>(aiString.length, aiString.data));
	}

	static GTSL::Vector3 aiVector3DToVector(const aiVector3D assimpVector) {
		return GTSL::Vector3(assimpVector.x, assimpVector.y, assimpVector.z);
	}

	static GTSL::Quaternion aiQuaternionToQuaternion(const aiQuaternion assimpQuaternion) {
		return GTSL::Quaternion(assimpQuaternion.x, assimpQuaternion.y, assimpQuaternion.z, assimpQuaternion.w);
	}


	
	void load(const GTSL::Buffer<BE::TAR>& sourceBuffer, GTSL::Buffer<BE::TAR>& meshDataBuffer);
};
