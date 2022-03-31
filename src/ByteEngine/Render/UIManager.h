#pragma once

#include "ByteEngine/Game/System.hpp"

#include <GTSL/Extent.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/FixedVector.hpp>
#include <GTSL/RGB.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Math/Vectors.hpp>
#include <GTSL/Tree.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Id.h"

#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/ApplicationManager.h"

enum class Alignments : uint8 {
			TOP,
	LEFT, CENTER, RIGHT,
			BOTTOM
};

/**
 * \brief Enumerates all ways an element can be scaled when resizing occurs.
 */
enum class ScalingPolicies : uint8 {
	/**
	 * \brief The size of the object is defined relative to screen size, which means that when a window or a parent element is resized that element will maintain it's size.
	 */
	FROM_SCREEN,
	/**
	 * \brief The size of the object is defined relative to another element which means that when a parent element is resized that element will change it's size.
	 */
	FROM_OTHER_ELEMENT,
	FROM_PARENT_CONTAINER = FROM_OTHER_ELEMENT
};

enum class SizingPolicies : uint8 {
	KEEP_CHILDREN_ASPECT_RATIO,
	SET_ASPECT_RATIO,
	FILL
};

/**
 * \brief Enumerates all ways to accomodate elements in a space.
 */
enum class SpacingPolicy : uint8 {
	/**
	 * \brief Places every object inside the element one next to each other.
	 */
	PACK,

	/**
	 * \brief Evenly distributes all objects inside the element.
	 */
	DISTRIBUTE
};

#undef NULL

class Square {
public:
	Square() = default;
	
	void SetColor(const Id newColor) { color = newColor; }
	[[nodiscard]] Id GetColor() const { return color; }
	
private:
	Id color;
	float32 rotation = 0.0f;
};

/**
 * \brief Manages ui elements.
 * All scales are defined and stored as a percentage of screen size. That is all constant size elements maintain the same percentage, and container relative elements have their scale updated every time they are scaled.
 */
class UIManager : public BE::System {
public:
	DECLARE_BE_TYPE(UIElement);

	explicit UIManager(const InitializeInfo& initializeInfo);


	struct PrimitiveData {
		enum class PrimitiveType { NULL, CANVAS, ORGANIZER, SQUARE, TEXT, CURVE } Type;
		GTSL::Vector2 RelativeLocation;
		GTSL::Vector2 AspectRatio;
		GTSL::Vector2 Bounds;
		GTSL::Vector2 Size;
		GTSL::Vector2 Position;
		Alignments Alignment;
		SizingPolicies SizingPolicy;
		ShaderGroupHandle Material;
		uint32 DerivedTypeIndex;
		ScalingPolicies ScalingPolicy;
		SpacingPolicy SpacingPolicy;
		bool isDirty;
		TaskHandle<UIElementHandle> OnHover, OnPress;
	};

	static EventHandle<UIElementHandle, PrimitiveData::PrimitiveType> GetOnCreateUIElementEventHandle() { return { u8"OnCreateUIElement" }; }

	UIElementHandle AddCanvas(const UIElementHandle ui_element_handle = UIElementHandle()) {
		//canvases.Emplace(system);
		auto canvasHandle =  add(ui_element_handle, PrimitiveData::PrimitiveType::CANVAS);
		getPrimitive(canvasHandle).AspectRatio = { 1280.0f / 720.0f, 1.0f };
		return canvasHandle;
	}

	auto& GetCanvases() { return canvases; }

	void AddColor(const Id colorName, const GTSL::RGBA color) { colors.Emplace(colorName, color); }
	[[nodiscard]] GTSL::RGBA GetColor(const Id color) const { return colors.At(color); }

	void SetExtent(const GTSL::Extent2D newExtent) { realExtent = newExtent; }

	UIElementHandle AddOrganizer(const UIElementHandle ui_element_handle = UIElementHandle()) {
		return add(ui_element_handle, PrimitiveData::PrimitiveType::ORGANIZER);
	}

	UIElementHandle AddSquare(const UIElementHandle element_handle = UIElementHandle()) {
		auto handle = add(element_handle, PrimitiveData::PrimitiveType::SQUARE);
		const auto place = squares.Emplace();
		auto& primitive = getPrimitive(handle);
		primitive.DerivedTypeIndex = place;
		return handle;
	}

	UIElementHandle AddText(const UIElementHandle element_handle, const GTSL::StringView text) {
		auto handle = add(element_handle, PrimitiveData::PrimitiveType::TEXT);
		const auto place = textPrimitives.Emplace(GetPersistentAllocator());
		auto& primitive = getPrimitive(handle);
		primitive.DerivedTypeIndex = place;
		textPrimitives[place].Text = text;
		return UIElementHandle(UIElementTypeIndentifier, place);
	}

	UIElementHandle AddCurve(const UIElementHandle element_handle) {
		auto handle = add(element_handle, PrimitiveData::PrimitiveType::CURVE);
		const auto place = curvePrimitives.Emplace(GetPersistentAllocator());
		auto& primitive = getPrimitive(handle);
		primitive.DerivedTypeIndex = place;
		auto& curve = curvePrimitives[place];
		return UIElementHandle(UIElementTypeIndentifier, place);
	}

	PrimitiveData& getPrimitive(const UIElementHandle element_handle) {
		return primitives[element_handle()];
	}

	[[nodiscard]] GTSL::Extent2D GetExtent() const { return realExtent; }

	GTSL::Result<UIElementHandle> FindPrimitiveUnderPoint(const GTSL::Vector2 point) {
		auto check = [&](decltype(primitives)::iterator level, auto&& self) -> GTSL::Result<UIElementHandle> {
			GTSL::Vector2 rect;
			if (GTSL::Math::Abs(rect - point) <= static_cast<const PrimitiveData&>(level).Bounds) { return GTSL::Result{ UIElementHandle(), true }; }

			for (auto e : level) {
				if (auto r = self(e, self)) { return r; }
			}
		};

		//check(primitives.begin(), check);

		return GTSL::Result<UIElementHandle>{ false };
	}

	void BindToElement(const UIElementHandle ui_element_handle, const TaskHandle<UIElementHandle> delegate) {
		getPrimitive(ui_element_handle).OnPress = delegate;
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

	void SetPosition(UIElementHandle ui_element_handle, GTSL::Vector2 pos) {
		auto& primitive = primitives[ui_element_handle()];
		switch (primitive.Type) {
		case PrimitiveData::PrimitiveType::NULL: break;
		case PrimitiveData::PrimitiveType::ORGANIZER: break;
		case PrimitiveData::PrimitiveType::SQUARE: break;
		}
		flagsAsDirty(ui_element_handle);
	}

	void SetAspectRatio(const UIElementHandle ui_element_handle, const GTSL::Vector2 aspectRatio) {
		auto& primitive = getPrimitive(ui_element_handle);
		primitive.AspectRatio = aspectRatio;
		flagsAsDirty(ui_element_handle);
	}

	void SetElementAlignment(const UIElementHandle organizer, Alignments alignment) {
		getPrimitive(organizer).Alignment = alignment;
		flagsAsDirty(organizer);
	}

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

	struct TextPrimitive {
		TextPrimitive(const BE::PAR& allocator) : Text(allocator) {}
		GTSL::String<BE::PAR> Text;
		GTSL::StaticString<64> Font{ u8"COOPBL" };
		bool IsLocalized = false;
	};

	auto GetRoot() { return primitives.begin(); }

private:
	GTSL::FixedVector<UIElementHandle, BE::PersistentAllocatorReference> canvases;
	GTSL::HashMap<Id, GTSL::RGBA, BE::PAR> colors;

	GTSL::Tree<PrimitiveData, BE::PAR> primitives;
	GTSL::FixedVector<Square, BE::PAR> squares;

	GTSL::FixedVector<TextPrimitive, BE::PAR> textPrimitives;

	struct CurvePrimitive {
		CurvePrimitive(const BE::PAR& allocator) : Points(3, allocator) {}
		GTSL::Vector<GTSL::Vector2, BE::PAR> Points;
	};
	GTSL::FixedVector<CurvePrimitive, BE::PAR> curvePrimitives;

	GTSL::Extent2D realExtent;

	GTSL::Vector<UIElementHandle, BE::PAR> queuedUpdates;

	UIElementHandle add(const UIElementHandle parent_handle, PrimitiveData::PrimitiveType type) {
		uint32 parentNodeKey = 0;

		if (parent_handle) {
			parentNodeKey = parent_handle();
		}

		auto primitiveIndex = primitives.Emplace(parentNodeKey);
		auto& primitive = primitives[primitiveIndex];
		primitive.Type = type;
		primitive.Alignment = Alignments::CENTER;
		primitive.Size = 1.0f;
		primitive.ScalingPolicy = ScalingPolicies::FROM_SCREEN;
		primitive.SizingPolicy = SizingPolicies::KEEP_CHILDREN_ASPECT_RATIO;
		primitive.SpacingPolicy = SpacingPolicy::DISTRIBUTE;
		primitive.AspectRatio = 1.f;
		primitive.DerivedTypeIndex = ~0u;
		primitive.isDirty = true;

		if (parent_handle) {
			flagsAsDirty(parent_handle); //if a child is added to an element it has to be re-evaluated
		}

		auto handle = GetApplicationManager()->MakeHandle<UIElementHandle>(UIElementTypeIndentifier, primitiveIndex);

		GetApplicationManager()->DispatchEvent(u8"UIManager", GetOnCreateUIElementEventHandle(), GTSL::MoveRef(handle), GTSL::MoveRef(primitive.Type));

		return handle;
	}

	void updateBranch(decltype(primitives)::iterator iterator);

	void flagsAsDirty(const UIElementHandle element_handle) {
		getPrimitive(element_handle).isDirty = true;
	}
};
