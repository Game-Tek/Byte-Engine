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

class AnimationResourceManager : ResourceManager
{
public:
	AnimationResourceManager() : ResourceManager("AnimationResourceManager")
	{
		initializePackageFiles(GetResourcePath(GTSL::StaticString<32>("Animations"), GTSL::ShortString<32>("bepkg")));
	}

	struct Bone
	{
		GTSL::Matrix4 Offset;
		uint32 AffectedBone[4];
		float32 EffectIntensity[4];

		INSERT_START(Bone)
		{
			Insert(insertInfo.Offset, buffer);
			Insert(insertInfo.AffectedBone, buffer);
			Insert(insertInfo.EffectIntensity, buffer);
		}

		EXTRACT_START(Bone)
		{
			Extract(extractInfo.Offset, buffer);
			Extract(extractInfo.AffectedBone, buffer);
			Extract(extractInfo.EffectIntensity, buffer);
		}
	};
	
	struct SkeletonData : Data
	{
		GTSL::Vector<Bone, BE::PAR> Bones;
		GTSL::FlatHashMap<Id, uint32, BE::PAR> BonesMap;
	};

	struct SkeletonDataSerialize : DataSerialize<SkeletonData>
	{
		INSERT_START(SkeletonDataSerialize)
		{
			INSERT_BODY;
			Insert(insertInfo.Bones, buffer);
			Insert(insertInfo.BonesMap, buffer);
		}

		EXTRACT_START(SkeletonDataSerialize)
		{
			EXTRACT_BODY;
			Extract(extractInfo.Bones, buffer);
			Extract(extractInfo.BonesMap, buffer);
		}
	};

	struct SkeletonInfo : Info<SkeletonDataSerialize>
	{
		//DECL_INFO_CONSTRUCTOR(SkeletonInfo, Info<SkeletonDataSerialize>);
	};
	
	struct AnimationData : Data
	{
		uint32 FrameCount, FPS;
		
		struct BoneAnimationData
		{
			GTSL::Vector3 Position; GTSL::Quaternion Rotation; GTSL::Vector3 Scale;

			INSERT_START(BoneAnimationData)
			{
				Insert(insertInfo.Position, buffer);
				Insert(insertInfo.Rotation, buffer);
				Insert(insertInfo.Scale, buffer);
			}

			EXTRACT_START(BoneAnimationData)
			{
				Extract(extractInfo.Position, buffer);
				Extract(extractInfo.Rotation, buffer);
				Extract(extractInfo.Scale, buffer);
			}
		};

		struct Frame
		{
			GTSL::Vector<BoneAnimationData, BE::PAR> Bones;

			INSERT_START(Frame)
			{
				Insert(insertInfo.Bones, buffer);
			}

			EXTRACT_START(Frame)
			{
				Extract(extractInfo.Bones, buffer);
			}
		};

		GTSL::Vector<Frame, BE::PAR> Frames;
	};

	struct AnimationDataSerialize : DataSerialize<AnimationData>
	{
		INSERT_START(AnimationDataSerialize)
		{
			INSERT_BODY;
			Insert(insertInfo.FrameCount, buffer);
			Insert(insertInfo.FPS, buffer);
			Insert(insertInfo.Frames, buffer);
		}

		EXTRACT_START(AnimationDataSerialize)
		{
			EXTRACT_BODY;
			Extract(extractInfo.FrameCount, buffer);
			Extract(extractInfo.FPS, buffer);
			Extract(extractInfo.Frames, buffer);
		}
	};

	struct AnimationInfo : Info<AnimationDataSerialize>
	{
		DECL_INFO_CONSTRUCTOR(AnimationInfo, Info<AnimationDataSerialize>);
	};

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


	
	void loadSkeleton(const GTSL::Buffer<BE::TAR>& sourceBuffer, SkeletonData& skeletonData, GTSL::Buffer<BE::TAR>& meshDataBuffer);
	void loadAnimation(const GTSL::Buffer<BE::TAR>& sourceBuffer, AnimationData& animationData, GTSL::Buffer<BE::TAR>& meshDataBuffer);
};
