#include "UIManager.h"

#include <GTSL/Math/Math.hpp>
#include <ByteEngine/Application/WindowSystem.hpp>

UIManager::UIManager(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"UIManager"),
colors(32, GetPersistentAllocator()), canvases(8, GetPersistentAllocator()), primitives(16, GetPersistentAllocator()), textPrimitives(8, GetPersistentAllocator()), curvePrimitives(8, GetPersistentAllocator()), queuedUpdates(8, GetPersistentAllocator()),
UIElementTypeIndentifier(GetApplicationManager()->RegisterType(this, u8"UIElement")), fonts(4, GetPersistentAllocator())
{
	GetApplicationManager()->AddEvent(u8"UIManager", GetOnCreateUIElementEventHandle());
}

void UIManager::ProcessUpdates() {
	auto screenExtent = GTSL::System::GetScreenExtent();
	auto screenSize = GTSL::Vector2(screenExtent.Width, screenExtent.Height);

	auto windowSystem = GetApplicationManager()->GetSystem<WindowSystem>(u8"WindowSystem");
	auto windowExtent = windowSystem->GetWindowClientExtent();
	auto windowSize = GTSL::Vector2(windowExtent.Width, windowExtent.Height);
	auto windowRelativeSize = GTSL::Vector2(windowSize.X() / windowSize.Y(), 1.0f);

	if (!(primitives.begin() != primitives.end())) { return; }

	auto screenRelativeSize = GTSL::Vector2(screenSize.X() / screenSize.Y(), 1.0f);

	UpdateData updateData{ screenRelativeSize, windowRelativeSize, screenRelativeSize * (windowSize / screenSize), windowSize / screenSize };

	primitives.operator[](1).HalfSize = windowRelativeSize;
	primitives.operator[](1).RenderSize = windowRelativeSize;

	updateBranch(primitives.begin(), &updateData, GTSL::Vector2(screenSize.X() / screenSize.Y(), 1.0f), GTSL::Vector2(), GTSL::Vector2());

	//auto result = FindPrimitiveUnderPoint({});
	//if (result) {
	//	GetApplicationManager()->AddStoredDynamicTask(getPrimitive(result.Get()).OnPress, UIElementHandle(result.Get()));
	//}

	//OnFontLoadTaskHandle = GetApplicationManager()->RegisterTask(this, u8"UIOnFontLoad", DependencyBlock(), &UIManager::OnFontLoad);
}

// -----------------------------------
//  All sizes relative to window size
// -----------------------------------

UIManager::PrimitiveData& UIManager::updateBranch(decltype(primitives)::iterator iterator, const UpdateData* update_data, GTSL::Vector2 size, GTSL::Vector2 start_position, GTSL::Vector2 parent_way) {
	const uint32 primitiveCount = iterator.GetLength();
	//if (!primitiveCount) { return; } // If this element has no children, do not evaluate.

	auto& primitive = static_cast<PrimitiveData&>(iterator);

	//if(!primitive.isDirty) { return; }

	// Distribution axis indicates which axis, (X = 0 || Y = 1) the distribution of the elements will occur in.
	uint32 distributionAxis = 0;

	// Distribution mask is used to cancel values in axis' where the calculations for the current distribution axis must not be taken into account.
	GTSL::Vector2 distributionMask = 0.0f;

	// Way indicates in which direction elements will be distributed among the different axis'.
	GTSL::Vector2 way = 0.0f, side = 0.0f;

	float32 w = 0, p = 1.0f;

	{ // Set distribution variables according to alignment policies.

		switch (primitive.Alignment) {
		case Alignments::LEFT: distributionAxis = 0; w = -1.0f; break;
		case Alignments::RIGHT: distributionAxis = 0; w = 1.0f; break;
		case Alignments::TOP: distributionAxis = 1; w = 1.0f; break;
		case Alignments::BOTTOM: distributionAxis = 1; w = -1.0f; break;
		case Alignments::CENTER: w = 0.0f; p = 0.0f; break;
		}

		way[distributionAxis] = -w;
		side[distributionAxis] = w;
		distributionMask[distributionAxis] = p;
	}

	GTSL::Vector2 halfSize;

	for(uint8 a = 0; a < 2; ++a) {
		switch (primitive.ScalingPolicies[a]) {
			case ScalingPolicies::FILL: {
				switch (primitive.SizingPolicies[a]) {
				case SizingPolicies::FROM_SCREEN: {
					if constexpr (WINDOW_SPACE) {
						halfSize[a] = update_data->WindowSize[a];
					} else {
				
					}
					break;
				}
				case SizingPolicies::FROM_OTHER_ELEMENT: {
					if constexpr (WINDOW_SPACE) {
						halfSize[a] = size[a];
					} else {
				
					}
					break;
				}
				}
				break;
			}
			case ScalingPolicies::SET_ASPECT_RATIO: {
				switch (primitive.SizingPolicies[a]) {
				case SizingPolicies::FROM_SCREEN: {
					if constexpr (WINDOW_SPACE) {
						halfSize[a] = primitive.HalfSize[a] / update_data->ScreenToWindowSize[a];
					} else {
				
					}
					break;
				}
				case SizingPolicies::FROM_OTHER_ELEMENT: {
					uint8 restrictingElementMinDimension = 0, elementMaxDimension = 0;

					if(size[0] < size[1]) {
						restrictingElementMinDimension = 0;
					} else {
						restrictingElementMinDimension = 1;
					}

					if(primitive.HalfSize[0] < primitive.HalfSize[1]) {
						elementMaxDimension = 0;
					} else {
						elementMaxDimension = 1;
					}

					float32 reductionFactor = size[restrictingElementMinDimension] / primitive.HalfSize[elementMaxDimension];

					if constexpr (WINDOW_SPACE) {
						halfSize[a] = primitive.HalfSize[a] * reductionFactor;
					} else {
						
					}
					break;
				}
				}
				break;
			}
			case ScalingPolicies::AUTO: {
				break;
			}
		}
	}

	primitive.RenderSize = halfSize;

	primitive.isDirty = false;

	if constexpr (WINDOW_SPACE) {
		primitive.Position = start_position + primitive.RenderSize * parent_way;
	}

	{
		uint32 i = 0;

		auto pos = primitive.Position + (halfSize - primitive.Padding) * side;

		for (auto e : iterator) {
			auto& n = updateBranch(e, update_data, halfSize - primitive.Padding, pos, way);
			pos += n.RenderSize * way * 2.0f + distributionMask * primitive.Spacing;

			++i;
		}
	}

	return primitive;
}