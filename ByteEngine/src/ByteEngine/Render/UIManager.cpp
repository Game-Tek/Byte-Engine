#include "UIManager.h"

#include <GTSL/Math/Math.hpp>

Canvas::Canvas() : Object("Canvas"), organizers(4, GetPersistentAllocator()), organizerDepth(4, GetPersistentAllocator()), organizerAspectRatios(4, GetPersistentAllocator()), squares(8, GetPersistentAllocator()),
                   primitives(8, GetPersistentAllocator()), organizersPrimitives(4, GetPersistentAllocator()), organizersPosition(4, GetPersistentAllocator()), organizerSizingPolicies(4, GetPersistentAllocator()),
                   organizerAlignments(4, GetPersistentAllocator())
{
	organizerTree.Initialize(GetPersistentAllocator());
}

uint16 Canvas::AddOrganizer(const Id name)
{
	auto organizer = organizerDepth.Emplace(0);
	organizerAspectRatios.Emplace();
	organizerAlignments.Emplace();
	organizerSizingPolicies.Emplace();
	organizerDepth.Emplace();
	organizersPosition.Emplace();
	organizersPrimitives.Emplace(4, GetPersistentAllocator());

	auto node = organizerTree.GetRootNode();
	node->Data = organizer;
	
	organizers.EmplaceAt(organizer, node);

	return organizer;
}

uint16 Canvas::AddOrganizer(const Id name, const uint16 parentOrganizer)
{
	auto organizer = organizerDepth.Emplace(0);
	organizerAspectRatios.Emplace();
	organizerAlignments.Emplace();
	organizerSizingPolicies.Emplace();
	organizerDepth.Emplace();
	organizersPosition.Emplace();
	organizersPrimitives.Emplace(4, GetPersistentAllocator());
	
	auto* child = organizerTree.AddChild(organizers[parentOrganizer]);
	child->Data = organizer;
	
	organizers.EmplaceAt(organizer, child);

	return organizer;
}

void Canvas::updateBranch(uint32 organizer)
{
	if (organizersPrimitives[organizer].GetLength())
	{
		auto orgAR = organizerAspectRatios[organizer]; auto orgLoc = organizersPosition[organizer];

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
			perPrimitiveInOrganizerAspectRatio.X = orgAR.X / organizersPrimitives[organizer].GetLength();
			perPrimitiveInOrganizerAspectRatio.Y = orgAR.Y;
			break;
			
		default: BE_ASSERT(false);
		}
		
		switch (organizerAlignments[organizer])
		{
		case Alignment::LEFT: break;
		case Alignment::CENTER: break;
		case Alignment::RIGHT: break;
		default: break;
		}

		GTSL::Vector2 startPos, increment;

		switch (organizerSizingPolicies[organizer].SpacingPolicy)
		{
		case SpacingPolicy::PACK:
		{
			startPos = { (-(orgAR.X * 0.5f) + (perPrimitiveInOrganizerAspectRatio.X * 0.5f)), orgLoc.Y };
			increment = perPrimitiveInOrganizerAspectRatio;
			increment.Y = 0;
				
			break;
		}
		case SpacingPolicy::DISTRIBUTE:
		{
			auto primCount = static_cast<float32>(organizersPrimitives[organizer].GetLength());
			auto primCount1 = primCount + 1.0f;
			auto freeArea = GTSL::Vector2(orgAR.X - (perPrimitiveInOrganizerAspectRatio.X * primCount), orgAR.Y - (perPrimitiveInOrganizerAspectRatio.Y * primCount));
			auto freeAreaPerPrim = freeArea / primCount1;
			startPos = { -(orgAR.X * 0.5f) + freeAreaPerPrim.X + perPrimitiveInOrganizerAspectRatio.X * 0.5f, orgLoc.Y };
			increment = perPrimitiveInOrganizerAspectRatio + freeAreaPerPrim;
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