#pragma once

#include <GTSL/HashMap.hpp>
#include <GTSL/Math/Matrix4.h>
#include <GTSL/Math/Quaternion.h>

#include "ResourceManager.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Application/AllocatorReferences.h"

class AnimationResourceManager : public ResourceManager
{
public:
	AnimationResourceManager();

	struct Bone
	{
		GTSL::Matrix4 Offset;
		GTSL::Vector<GTSL::Pair<uint32, float32>, BE::PAR> AffectedVertices;

		INSERT_START(Bone)
		{
			Insert(insertInfo.Offset, buffer);
			Insert(insertInfo.AffectedVertices, buffer);
		}

		EXTRACT_START(Bone)
		{
			Extract(extractInfo.Offset, buffer);
			Extract(extractInfo.AffectedVertices, buffer);
		}

		Bone(const BE::PAR& allocator) : AffectedVertices(allocator) {}
	};
	
	struct SkeletonData : Data
	{
		GTSL::Vector<Bone, BE::PAR> Bones;
		GTSL::HashMap<Id, uint32, BE::PAR> BonesMap;
		
		SkeletonData(const BE::PAR& allocator) : Bones(allocator), BonesMap(256, 0.1f, allocator) {}
	};

	struct SkeletonDataSerialize : SkeletonData
	{
		SkeletonDataSerialize(const BE::PAR& allocator) : SkeletonData(allocator) {}

		uint32 ByteOffset = 0;
		
		INSERT_START(SkeletonDataSerialize) {
			INSERT_BODY;
			Insert(insertInfo.Bones, buffer);
			Insert(insertInfo.BonesMap, buffer);
		}

		EXTRACT_START(SkeletonDataSerialize) {
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
		uint32 FrameCount = 0, FPS = 0;
		
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
			Frame(const BE::PAR& allocator) : Bones(allocator) {}
			
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

		AnimationData(const BE::PAR& allocator) : Frames(allocator) {}
	};

	struct AnimationDataSerialize : AnimationData
	{
		uint32 ByteOffset = 0;
		
		INSERT_START(AnimationDataSerialize)
		{
			INSERT_BODY;
			Insert(insertInfo.FrameCount, buffer);
			Insert(insertInfo.FPS, buffer);
			//Insert(insertInfo.Frames, buffer);
		}

		EXTRACT_START(AnimationDataSerialize)
		{
			EXTRACT_BODY;
			Extract(extractInfo.FrameCount, buffer);
			Extract(extractInfo.FPS, buffer);
			//Extract(extractInfo.Frames, buffer);
		}

		AnimationDataSerialize(const BE::PAR& allocator) : AnimationData(allocator) {}
	};

	struct AnimationInfo : Info<AnimationDataSerialize>
	{
		DECL_INFO_CONSTRUCTOR(AnimationInfo, Info<AnimationDataSerialize>);
	};

private:
	void loadSkeleton(const GTSL::Range<const byte*> sourceBuffer, SkeletonData& skeletonData, GTSL::Buffer<BE::TAR>& meshDataBuffer);
	void loadAnimation(const GTSL::Range<const byte*> sourceBuffer, AnimationData& animationData, GTSL::Buffer<BE::TAR>& meshDataBuffer);

	GTSL::HashMap<Id, AnimationDataSerialize, BE::PAR> animations;
	GTSL::StaticVector<GTSL::File, MAX_THREADS> packageFiles;
};
