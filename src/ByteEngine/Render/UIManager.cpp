#include "UIManager.h"

#include <GTSL/Math/Math.hpp>

UIManager::UIManager(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"UIManager"),
colors(32, GetPersistentAllocator()), canvases(8, GetPersistentAllocator()), primitives(16, GetPersistentAllocator()), squares(8, GetPersistentAllocator()), textPrimitives(8, GetPersistentAllocator()), curvePrimitives(8, GetPersistentAllocator()), queuedUpdates(8, GetPersistentAllocator()),
UIElementTypeIndentifier(GetApplicationManager()->RegisterType(this, u8"UIElement"))
{
	GetApplicationManager()->AddEvent(u8"UIManager", GetOnCreateUIElementEventHandle());
}

void UIManager::ProcessUpdates() {
	auto screenExtent = GTSL::System::GetScreenExtent();
	auto screenSize = GTSL::Vector2(screenExtent.Width, screenExtent.Height);

	auto windowSize = GTSL::Vector2(1280.0f, 720.f);

	if (primitives.begin().GetPosition() == 1) { return; }

	updateBranch(primitives.begin());

	//auto result = FindPrimitiveUnderPoint({});
	//if (result) {
	//	GetApplicationManager()->AddStoredDynamicTask(getPrimitive(result.Get()).OnPress, UIElementHandle(result.Get()));
	//}
}

void UIManager::updateBranch(decltype(primitives)::iterator iterator) {
	const uint32 primitiveCount = iterator.GetLength();
	if (!primitiveCount) { return; } // If this element has no children, do not evaluate.

	auto& primitive = static_cast<PrimitiveData&>(iterator);

	const GTSL::Vector2 halfSize = primitive.HalfSize;

	// Distribution axis indicates which axis, (X = 0 || Y = 1) the distribution of the elements will occur in.
	uint32 distributionAxis = 0;

	// Distribution mask is used to cancel values in axis' where the calculations for the current distribution axis must not be taken into account.
	GTSL::Vector2 distributionMask = 0;

	// Way indicates in which direction elements will be distributed among the different axis'.
	GTSL::Vector2 way = 0.0f;

	{ // Set distribution variables according to alignment policies.
		float32 w = 0;

		switch (primitive.Alignment) {
		case Alignments::LEFT: distributionAxis = 0; w = -1.0f; break;
		case Alignments::RIGHT: distributionAxis = 0; w = 1.0f; break;
		case Alignments::TOP: distributionAxis = 1; w = 1.0f; break;
		case Alignments::BOTTOM: distributionAxis = 1; w = -1.0f; break;
		case Alignments::CENTER: w = 0.0f; break;
		}

		way[distributionAxis] = w;
		distributionMask[distributionAxis] = 1.0f;
	}

	// Maximum half size each primitive can have, when the available space is divided equally amongst all children taking into account distribution axis'.
	GTSL::Vector2 perPrimitiveHalfSize = halfSize;
	perPrimitiveHalfSize[distributionAxis] = halfSize[distributionAxis] / static_cast<float32>(primitiveCount);

	// Starting position for elements inside this element. A delta will be added to each primitive to correctly distribute them inside this element.
	const GTSL::Vector2 startPosition = GTSL::Vector2() - perPrimitiveHalfSize * (primitiveCount + 1) / 2.0f * distributionMask;
	// How much each element has to move to correctly distribute them. Each of the children's positions will be the sum of the starting position plus a multiple of this increment times the child index.
	const GTSL::Vector2 increment = perPrimitiveHalfSize * 2.0f * distributionMask;

	//switch (primitive.SpacingPolicy) {
	//case SpacingPolicy::PACK: {
	//	startPos = { ((extent.X() * 0.5f * way) + (perPrimitiveInOrganizerAspectRatio.X() * 0.5f * (-way))), orgLoc.Y() };
	//	increment = perPrimitiveInOrganizerAspectRatio * (-way);
	//	increment.Y() = 0;
	//
	//	break;
	//}
	//case SpacingPolicy::DISTRIBUTE: {
	//	auto freeArea = GTSL::Vector2(extent.X() - (perPrimitiveInOrganizerAspectRatio.X() * primCount), extent.Y() - (perPrimitiveInOrganizerAspectRatio.Y() * primCount));
	//	auto freeAreaPerPrim = freeArea / (primCount + 1.0f);
	//	startPos = { (extent.X() * 0.5f * way) + ((freeAreaPerPrim.X() + perPrimitiveInOrganizerAspectRatio.X() * 0.5f) * (-way)), orgLoc.Y() };
	//	increment = (perPrimitiveInOrganizerAspectRatio + freeAreaPerPrim) * (-way);
	//	increment.Y() = 0;
	//
	//	break;
	//}
	//}

	primitive.isDirty = false;

	{
		uint32 i = 0;
		for (auto e : iterator) {
			auto& f = static_cast<PrimitiveData&>(e);
			f.Position = startPosition + increment * i;
			f.HalfSize[distributionAxis] = perPrimitiveHalfSize[distributionAxis];
			updateBranch(e);
			++i;
		}
	}
}

inline void size(float32 dpc, GTSL::Vector2 size, GTSL::Vector2 screen_resolution) {
	// size is a fraction of screen size
}