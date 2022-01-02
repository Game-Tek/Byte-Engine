#pragma once

#include "ByteEngine/Game/System.hpp"

#include <GTSL/Extent.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/FixedVector.hpp>
#include <GTSL/RGB.h>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vectors.hpp>
#include <GTSL/Tree.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/ApplicationManager.h"

MAKE_HANDLE(uint32, Canvas);

enum class Alignments : uint8 {
			TOP,
	LEFT, CENTER, RIGHT,
			BOTTOM
};

enum class ScalingPolicies : uint8 {
	FROM_SCREEN, FROM_OTHER_CONTAINER
};

enum class SizingPolicies : uint8 {
	KEEP_CHILDREN_ASPECT_RATIO,
	SET_ASPECT_RATIO,
	FILL
};

enum class SpacingPolicy : uint8
{
	PACK, DISTRIBUTE
};

class Button : public Object
{
public:
	
private:
};

#undef NULL

MAKE_HANDLE(uint32, UIElement);

struct PrimitiveData {
	enum class PrimitiveType { NULL, ORGANIZER, SQUARE } Type;
	GTSL::Vector2 RelativeLocation;
	GTSL::Vector2 AspectRatio;
	GTSL::Vector2 Bounds;
	Alignments Alignment = Alignments::CENTER;
	SizingPolicies SizingPolicy = SizingPolicies::SET_ASPECT_RATIO;
	ShaderGroupHandle Material;
	uint32 DerivedTypeIndex = 0u;
	ScalingPolicies ScalingPolicy;
	SpacingPolicy SpacingPolicy;
	bool isDirty = false;
	DynamicTaskHandle<UIElementHandle> OnHover, OnPress;
};

class Square {
public:
	Square() = default;
	
	void SetColor(const Id newColor) { color = newColor; }
	[[nodiscard]] Id GetColor() const { return color; }
	
private:
	Id color;
	float32 rotation = 0.0f;
};

class Canvas : public BE::System {
public:
	Canvas();

	void SetExtent(const GTSL::Extent2D newExtent) { realExtent = newExtent; }

	UIElementHandle AddOrganizer(const UIElementHandle ui_element_handle = UIElementHandle(0)) {
		auto primitiveIndex = primitives.Emplace(ui_element_handle());
		return UIElementHandle(primitiveIndex);
	}

	UIElementHandle AddSquare(const UIElementHandle element_handle = UIElementHandle(0)) {
		const auto primitiveIndex = primitives.Emplace(element_handle());
		const auto place = squares.Emplace();
		auto& primitive = primitives[primitiveIndex];
		primitive.AspectRatio = 1.f;
		primitive.DerivedTypeIndex = primitiveIndex;
		flagsAsDirty(element_handle);
		return UIElementHandle(place);
	}

	UIElementHandle AddText(const UIElementHandle element_handle, const GTSL::StringView text) {
		const auto primitiveIndex = primitives.Emplace(element_handle());
		const auto place = textPrimitives.Emplace(GetPersistentAllocator());
		auto& primitive = getPrimitive(UIElementHandle(primitiveIndex));
		primitive.AspectRatio = 1.f;
		primitive.DerivedTypeIndex = primitiveIndex;
		flagsAsDirty(element_handle);
		textPrimitives[place].Text = text;
		return UIElementHandle(place);
	}

	UIElementHandle AddCurve(const UIElementHandle element_handle) {
		const auto primitiveIndex = primitives.Emplace(element_handle());
		const auto place = curvePrimitives.Emplace(GetPersistentAllocator());
		auto& primitive = getPrimitive(UIElementHandle(primitiveIndex));
		primitive.AspectRatio = 1.f;
		primitive.DerivedTypeIndex = primitiveIndex;
		flagsAsDirty(element_handle);
		auto& curve = curvePrimitives[place];
		return UIElementHandle(place);
	}

	void BindToElement(const UIElementHandle ui_element_handle, const DynamicTaskHandle<UIElementHandle> delegate) {
		getPrimitive(ui_element_handle).OnPress = delegate;
	}

	void SetAspectRatio(const UIElementHandle ui_element_handle, const GTSL::Vector2 aspectRatio) {
		auto& primitive = primitives[ui_element_handle()];
		primitive.AspectRatio = aspectRatio;
		flagsAsDirty(ui_element_handle);
	}

	void SetColor(const UIElementHandle ui_element_handle, const Id color) {
		auto& primitive = primitives[ui_element_handle()];
		switch (primitive.Type) {
		case PrimitiveData::PrimitiveType::NULL: break;
		case PrimitiveData::PrimitiveType::ORGANIZER: break;
		case PrimitiveData::PrimitiveType::SQUARE: squares[primitive.DerivedTypeIndex].SetColor(color); break;
		}
	}

	void SetMaterial(const UIElementHandle ui_element_handle, const ShaderGroupHandle material) {
		getPrimitive(ui_element_handle).Material = material;
		flagsAsDirty(ui_element_handle);
	}

	PrimitiveData& getPrimitive(const UIElementHandle element_handle) {
		return primitives[element_handle()];
	}

	void SetOrganizerAlignment(const UIElementHandle organizer, Alignments alignment) {
		getPrimitive(organizer).Alignment = alignment;
		flagsAsDirty(organizer);
	}

	[[nodiscard]] GTSL::Extent2D GetExtent() const { return realExtent; }

	GTSL::Result<UIElementHandle> FindPrimitiveUnderPoint(const GTSL::Vector2 point) {
		auto check = [&](decltype(primitives)::iterator level, auto&& self) -> GTSL::Result<UIElementHandle> {
			GTSL::Vector2 rect;
			if (GTSL::Math::Abs(rect - point) <= static_cast<const PrimitiveData&>(level).Bounds) { return GTSL::Result{ UIElementHandle(), true }; }
		
			for(auto e : level) {
				if (auto r = self(e, self)) { return r; }
			}
		};

		check(primitives.begin(), check);

		return GTSL::Result<UIElementHandle>{ false };
	}
	
	void SetPosition(UIElementHandle ui_element_handle, GTSL::Vector2 pos) {
		auto& primitive = primitives[ui_element_handle()];
		switch (primitive.Type) {
		case PrimitiveData::PrimitiveType::NULL: break;
		case PrimitiveData::PrimitiveType::ORGANIZER: break;
		case PrimitiveData::PrimitiveType::SQUARE: break;
		}
		flagsAsDirty(ui_element_handle);
	}
	
	//void NestElements(UIElementHandle parent_handle, UIElementHandle child_handle) {
	//	//organizersPrimitives[organizer()].EmplaceBack(squares[square].PrimitiveIndex);
	//	flagsAsDirty(parent_handle);
	//}

	void SetElementSizingPolicy(UIElementHandle organizer, SizingPolicies sizingPolicy) {
		getPrimitive(organizer).SizingPolicy = sizingPolicy;
		flagsAsDirty(organizer);
	}

	void SetElementScalingPolicy(UIElementHandle organizer, ScalingPolicies scalingPolicy) {
		getPrimitive(organizer).ScalingPolicy = scalingPolicy;
		flagsAsDirty(organizer);
	}

	void SetElementSpacingPolicy(UIElementHandle organizer, SpacingPolicy spacingPolicy) {
		getPrimitive(organizer).SpacingPolicy = spacingPolicy;
		flagsAsDirty(organizer);
	}

	void ProcessUpdates();
	
private:
	GTSL::Tree<PrimitiveData, BE::PAR> primitives;
	GTSL::FixedVector<Square, BE::PAR> squares;

	struct TextPrimitive {
		TextPrimitive(const BE::PAR& allocator) : Text(allocator) {}
		GTSL::String<BE::PAR> Text;
		GTSL::StaticString<64> Font{ u8"COOPBL" };
	};
	GTSL::FixedVector<TextPrimitive, BE::PAR> textPrimitives;

	struct CurvePrimitive {
		CurvePrimitive(const BE::PAR& allocator) : Points(3, allocator) {}
		GTSL::Vector<GTSL::Vector2, BE::PAR> Points;
	};
	GTSL::FixedVector<CurvePrimitive, BE::PAR> curvePrimitives;
	
	GTSL::Extent2D realExtent;

	GTSL::Vector<UIElementHandle, BE::PAR> queuedUpdates;

	/**
	 * \brief Queues an organizer update to a list and prunes any redundant children updates if a parent is already updating higher up in the hierarchy.
	 * \param organizer organizer to update from
	 */
	void queueUpdateAndCull(UIElementHandle organizer);
	
	void updateBranch(UIElementHandle ui_element_handle = UIElementHandle(0));

	void flagsAsDirty(const UIElementHandle element_handle) {
		getPrimitive(element_handle).isDirty = true;
	}
};

class UIManager : public BE::System {
public:
	explicit UIManager(const InitializeInfo& initializeInfo);

	void AddCanvas(const CanvasHandle system)
	{
		canvases.Emplace(system);
	}

	auto& GetCanvases() { return canvases; }

	void AddColor(const Id colorName, const GTSL::RGBA color) { colors.Emplace(colorName, color); }
	[[nodiscard]] GTSL::RGBA GetColor(const Id color) const { return colors.At(color); }

private:
	GTSL::FixedVector<CanvasHandle, BE::PersistentAllocatorReference> canvases;
	GTSL::HashMap<Id, GTSL::RGBA, BE::PAR> colors;
};
