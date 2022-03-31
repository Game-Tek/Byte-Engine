#include "UIManager.h"

#include <GTSL/Math/Math.hpp>

UIManager::UIManager(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"UIManager"),
colors(32, GetPersistentAllocator()), canvases(8, GetPersistentAllocator()), primitives(16, GetPersistentAllocator()), squares(8, GetPersistentAllocator()), textPrimitives(8, GetPersistentAllocator()), curvePrimitives(8, GetPersistentAllocator()), queuedUpdates(8, GetPersistentAllocator()),
UIElementTypeIndentifier(GetApplicationManager()->RegisterType(this, u8"UIElement"))
{
	GetApplicationManager()->AddEvent(u8"UIManager", GetOnCreateUIElementEventHandle());
}

void UIManager::ProcessUpdates() {
	updateBranch(UIElementHandle(UIElementTypeIndentifier, 1));

	//auto result = FindPrimitiveUnderPoint({});
	//if (result) {
	//	GetApplicationManager()->AddStoredDynamicTask(getPrimitive(result.Get()).OnPress, UIElementHandle(result.Get()));
	//}
}

void UIManager::updateBranch(UIElementHandle ui_element_handle) {
	//for (uint32 i = 0; i < organizersPerOrganizer[organizer()].GetLength(); ++i) { updateBranch(organizersPerOrganizer[organizer()][i]); }
//
//if (!organizersPrimitives[organizer()].GetLength()) { return; }
//
//auto primCount = static_cast<float32>(organizersPrimitives[organizer()].GetLength());
//
//auto orgAR = primitives[organizersAsPrimitives[organizer()]].AspectRatio; auto orgLoc = primitives[organizersAsPrimitives[organizer()]].RelativeLocation;

	GTSL::Vector2 orgAR;
	float32 way = 0.0f;

	auto& primitive = getPrimitive(ui_element_handle);

	switch (primitive.Alignment) { case Alignments::LEFT: way = -1.0f; break; case Alignments::CENTER: way = 0.0f; break; case Alignments::RIGHT: way = 1.0f; break; }

	GTSL::Vector2 perPrimitiveInOrganizerAspectRatio;

	switch (primitive.SizingPolicy) {
	case SizingPolicies::KEEP_CHILDREN_ASPECT_RATIO: {
		const auto minDimension = GTSL::Math::Min(orgAR.X(), orgAR.Y());
		perPrimitiveInOrganizerAspectRatio = { minDimension, minDimension };

		break;
	}
	case SizingPolicies::FILL: {
		uint32 distributionAxis = 0;

		switch (primitive.Alignment) {
		case Alignments::LEFT:
		case Alignments::RIGHT:
			distributionAxis = 0;
			break;

		case Alignments::TOP:
		case Alignments::BOTTOM:
			distributionAxis = 1;
			break;

		case Alignments::CENTER:
			BE_ASSERT(false);
		default: break;
		}

		perPrimitiveInOrganizerAspectRatio[distributionAxis] = orgAR[distributionAxis] / 1;//organizersPrimitives[organizer()].GetLength();
		perPrimitiveInOrganizerAspectRatio[(distributionAxis + 1) % 2] = orgAR[(distributionAxis + 1) % 2];

		break;
	}
	case SizingPolicies::SET_ASPECT_RATIO: {
		const auto minDimension = GTSL::Math::Min(orgAR.X(), orgAR.Y());
		perPrimitiveInOrganizerAspectRatio = { minDimension, minDimension };

		break;
	}

	default: BE_ASSERT(false);
	}

	GTSL::Vector2 startPos, increment, orgLoc; uint32 primCount = 0;

	switch (primitive.SpacingPolicy) {
	case SpacingPolicy::PACK: {
		startPos = { ((orgAR.X() * 0.5f * way) + (perPrimitiveInOrganizerAspectRatio.X() * 0.5f * (-way))), orgLoc.Y() };
		increment = perPrimitiveInOrganizerAspectRatio * (-way);
		increment.Y() = 0;

		break;
	}
	case SpacingPolicy::DISTRIBUTE: {
		auto freeArea = GTSL::Vector2(orgAR.X() - (perPrimitiveInOrganizerAspectRatio.X() * primCount), orgAR.Y() - (perPrimitiveInOrganizerAspectRatio.Y() * primCount));
		auto freeAreaPerPrim = freeArea / (primCount + 1.0f);
		startPos = { (orgAR.X() * 0.5f * way) + ((freeAreaPerPrim.X() + perPrimitiveInOrganizerAspectRatio.X() * 0.5f) * (-way)), orgLoc.Y() };
		increment = (perPrimitiveInOrganizerAspectRatio + freeAreaPerPrim) * (-way);
		increment.Y() = 0;

		break;
	}
	}

	primitive.isDirty = false;

	//for (uint32 i = 0; i < organizersPrimitives.GetLength(); ++i) {
	//	primitives[organizersPrimitives[organizer()][i]].AspectRatio = perPrimitiveInOrganizerAspectRatio;
	//	primitives[organizersPrimitives[organizer()][i]].RelativeLocation = startPos;
	//
	//	startPos += increment;
	//}
}
