#include "UIManager.h"

#include <GTSL/Math/Math.hpp>

Canvas::Canvas() : Object("Canvas"), organizers(4, GetPersistentAllocator()), organizerDepth(4, GetPersistentAllocator()), organizersAsPrimitives(4, GetPersistentAllocator()),
squares(8, GetPersistentAllocator()), primitives(8, GetPersistentAllocator()), organizersPrimitives(4, GetPersistentAllocator()),
organizerSizingPolicies(4, GetPersistentAllocator()), organizerAlignments(4, GetPersistentAllocator()), organizersPerOrganizer(4, GetPersistentAllocator()),
queuedUpdates(8, GetPersistentAllocator())
{
	organizerTree.Initialize(GetPersistentAllocator());
}

uint16 Canvas::AddOrganizer(const Id name)
{
	auto organizer = organizersAsPrimitives.Emplace(primitives.Emplace());
	organizerDepth.Emplace(0);
	organizerAlignments.Emplace(Alignment::CENTER);
	organizerSizingPolicies.Emplace(SizingPolicy::SET_ASPECT_RATIO);
	organizersPrimitives.Emplace(4, GetPersistentAllocator());
	organizersPerOrganizer.Emplace(4, GetPersistentAllocator());

	auto node = organizerTree.GetRootNode();
	node->Data = organizer;
	
	organizers.EmplaceAt(organizer, node);

	return organizer;
}

uint16 Canvas::AddOrganizer(const Id name, const uint16 parentOrganizer)
{
	auto organizer = organizersAsPrimitives.Emplace(0);
	organizerDepth.Emplace(0);
	organizerAlignments.Emplace(Alignment::CENTER);
	organizerSizingPolicies.Emplace(SizingPolicy::SET_ASPECT_RATIO);
	organizersPrimitives.Emplace(4, GetPersistentAllocator());
	organizersPerOrganizer.Emplace(4, GetPersistentAllocator());
	
	auto* child = organizerTree.AddChild(organizers[parentOrganizer]);
	child->Data = organizer;
	
	organizers.EmplaceAt(organizer, child);

	return organizer;
}

void Canvas::ProcessUpdates()
{
	for(auto e : queuedUpdates) { updateBranch(e); }
	queuedUpdates.ResizeDown(0);
}

void Canvas::queueUpdateAndCull(uint32 organizer)
{
	GTSL::Array<uint32, 32> branchesToProne;

	uint32 i = 0;
	for(auto e : queuedUpdates)
	{
		if(organizerDepth[organizer] < organizerDepth[e]) { branchesToProne.EmplaceBack(i); }
		
		++i;
	}

	for(auto e : branchesToProne) { queuedUpdates.Pop(e); }
	queuedUpdates.EmplaceBack(organizer);
}

void Canvas::updateBranch(uint32 organizer)
{
	for (uint32 i = 0; i < organizersPerOrganizer[organizer].GetLength(); ++i) { updateBranch(organizersPerOrganizer[organizer][i]); }
	
	if (organizersPrimitives[organizer].GetLength())
	{
		auto primCount = static_cast<float32>(organizersPrimitives[organizer].GetLength());
		
		auto orgAR = primitives[organizersAsPrimitives[organizer]].AspectRatio; auto orgLoc = primitives[organizersAsPrimitives[organizer]].RelativeLocation;

		float32 way = 1.0f;

		switch (organizerAlignments[organizer])
		{
		case Alignment::LEFT: way = -1.0f; break;
		case Alignment::CENTER: way = 0.0f; break;
		case Alignment::RIGHT: way = 1.0f; break;
		default: break;
		}
		
		GTSL::Vector2 perPrimitiveInOrganizerAspectRatio;

		switch (organizerSizingPolicies[organizer].SizingPolicy)
		{
		case SizingPolicy::KEEP_CHILDREN_ASPECT_RATIO:
		{
			const auto minDimension = GTSL::Math::Min(orgAR.X, orgAR.Y);
			perPrimitiveInOrganizerAspectRatio = { minDimension, minDimension };

			break;
		}
			
		case SizingPolicy::FILL:
			switch (organizerAlignments[organizer])
			{
				case Alignment::LEFT:
				case Alignment::RIGHT:
					perPrimitiveInOrganizerAspectRatio.X = orgAR.X / organizersPrimitives[organizer].GetLength();
					perPrimitiveInOrganizerAspectRatio.Y = orgAR.Y;
					break;

				case Alignment::TOP:
				case Alignment::BOTTOM:
					perPrimitiveInOrganizerAspectRatio.X = orgAR.X;
					perPrimitiveInOrganizerAspectRatio.Y = orgAR.Y / organizersPrimitives[organizer].GetLength();
					break;				
				
				case Alignment::CENTER:
					BE_ASSERT(false);
				default: break;
			}
			break;
			
		case SizingPolicy::SET_ASPECT_RATIO:
		{
			const auto minDimension = GTSL::Math::Min(orgAR.X, orgAR.Y);
			perPrimitiveInOrganizerAspectRatio = { minDimension, minDimension };

			break;
		}
			
		default: BE_ASSERT(false);
		}

		GTSL::Vector2 startPos, increment;

		switch (organizerSizingPolicies[organizer].SpacingPolicy)
		{
		case SpacingPolicy::PACK:
		{
			startPos = { ((orgAR.X * 0.5f * way) + (perPrimitiveInOrganizerAspectRatio.X * 0.5f * (-way))), orgLoc.Y };
			increment = perPrimitiveInOrganizerAspectRatio * (-way);
			increment.Y = 0;
				
			break;
		}
		case SpacingPolicy::DISTRIBUTE:
		{
			auto primCount1 = primCount + 1.0f;
			auto freeArea = GTSL::Vector2(orgAR.X - (perPrimitiveInOrganizerAspectRatio.X * primCount), orgAR.Y - (perPrimitiveInOrganizerAspectRatio.Y * primCount));
			auto freeAreaPerPrim = freeArea / primCount1;
			startPos = { (orgAR.X * 0.5f * way) + ((freeAreaPerPrim.X + perPrimitiveInOrganizerAspectRatio.X * 0.5f) * (-way)), orgLoc.Y };
			increment = (perPrimitiveInOrganizerAspectRatio + freeAreaPerPrim) * (-way);
			increment.Y = 0;
				
			break;
		}
		}
		
		for (uint32 i = 0; i < organizersPrimitives[organizer].GetLength(); ++i)
		{
			primitives[organizersPrimitives[organizer][i]].AspectRatio = perPrimitiveInOrganizerAspectRatio;
			primitives[organizersPrimitives[organizer][i]].RelativeLocation = startPos;

			startPos += increment;
		}
	}
}

void CanvasSystem::Initialize(const InitializeInfo& initializeInfo)
{
	canvases.Initialize(8, GetPersistentAllocator());
}

void CanvasSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

void UIManager::Initialize(const InitializeInfo& initializeInfo)
{
	canvases.Initialize(8, GetPersistentAllocator());
	colors.Initialize(16, GetPersistentAllocator());
}

void UIManager::Shutdown(const ShutdownInfo& shutdownInfo)
{
}