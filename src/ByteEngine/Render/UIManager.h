#pragma once

#include <GTSL/Extent.h>
#include <GTSL/FixedVector.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/RGB.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Tree.hpp>
#include <GTSL/Math/Vectors.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Id.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Game/System.hpp"
#include "ByteEngine/Resources/FontResourceManager.h"
#include "GTSL/JSON.hpp"

enum class Alignments : uint8 {
			TOP,
	LEFT, CENTER, RIGHT,
			BOTTOM
};

/**
 * \brief Enumerates all ways an element can be sized when resizing occurs.
 */
enum class SizingPolicies : uint8 {
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


/**
 * \brief All ways an en element can be scaled to fit inside it's parent.
 */
enum class ScalingPolicies : uint8 {
	FILL,
	SET_ASPECT_RATIO,
	AUTO
};

/**
 * \brief Enumerates all ways to accomodate elements in a space. This can be further by the alignment.
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

/**
 * \brief Manages ui elements.
 * All scales are defined and stored as a percentage of screen size. That is all constant size elements maintain the same percentage, and container relative elements have their scale updated every time they are scaled.
 */
class UIManager : public BE::System {
public:
	DECLARE_BE_TYPE(UIElement);

	DECLARE_BE_TASK(OnFontLoad, BE_RESOURCES(), FontResourceManager::FontData, GTSL::Buffer<BE::PAR>);

	explicit UIManager(const InitializeInfo& initializeInfo);

	static constexpr bool WINDOW_SPACE = true;

	struct PrimitiveData {
		enum class PrimitiveType { NONE, CANVAS, ORGANIZER, SQUARE, TEXT, CURVE } Type;
		GTSL::Vector2 RelativeLocation;
		GTSL::Vector2 Bounds;
		GTSL::Vector2 HalfSize, RenderSize;
		//Relative to parent.
		GTSL::Vector2 Position;
		Alignments Alignment;
		ScalingPolicies ScalingPolicies[2/*DIMENSIONS*/];
		SizingPolicies SizingPolicies[2];
		RenderModelHandle Material;
		uint32 DerivedTypeIndex;
		SpacingPolicy SpacingPolicy;
		bool isDirty;
		TaskHandle<UIElementHandle> OnHover, OnPress;
		GTSL::Vector4 Color;
		float32 Rounding = 0.0f, Padding, Spacing;
	};

	static EventHandle<UIElementHandle, PrimitiveData::PrimitiveType> GetOnCreateUIElementEventHandle() { return { u8"OnCreateUIElement" }; }

	auto& GetCanvases() { return canvases; }

	void AddColor(const Id colorName, const GTSL::RGBA color) { colors.Emplace(colorName, color); }
	[[nodiscard]] GTSL::RGBA GetColor(const Id color) const { return colors.At(color); }

	void SetExtent(const GTSL::Extent2D newExtent) { realExtent = newExtent; }

	void SetScale(UIElementHandle element_handle, const uint8 axis, float32 scale) {
		getPrimitive(element_handle).HalfSize[axis] = scale;
		flagsAsDirty(element_handle);
	}

	/**
	 * \brief Sets the scaling percentage for a UI element. This scale will be calculated based on the sizing policy.
	 * \param element_handle Element to set scale for.
	 * \param scale Scaling percentage.
	 */
	void SetScale(UIElementHandle element_handle, GTSL::Vector2 scale) {
		getPrimitive(element_handle).HalfSize = scale;
		flagsAsDirty(element_handle);
	}

	void SetRounding(const UIElementHandle element_handle, const float32 rounding) {
		getPrimitive(element_handle).Rounding = rounding;
		flagsAsDirty(element_handle);
	}

	void SetPadding(const UIElementHandle element_handle, const float32 padding) {
		getPrimitive(element_handle).Padding = padding;
		flagsAsDirty(element_handle);
	}

	void SetSpacing(const UIElementHandle element_handle, const float32 spacing) {
		getPrimitive(element_handle).Spacing = spacing;
		flagsAsDirty(element_handle);
	}

	UIElementHandle AddCanvas(const UIElementHandle ui_element_handle = UIElementHandle()) {
		//canvases.Emplace(system);
		auto canvasHandle = add(ui_element_handle, PrimitiveData::PrimitiveType::CANVAS);
		SetScalingPolicy(canvasHandle, 0, ScalingPolicies::FILL);
		SetScalingPolicy(canvasHandle, 1, ScalingPolicies::FILL);
		SetSizingPolicy(canvasHandle, 0, SizingPolicies::FROM_SCREEN);
		SetSizingPolicy(canvasHandle, 1, SizingPolicies::FROM_SCREEN);
		return canvasHandle;
	}

	void SetString(const UIElementHandle ui_element_handle, const GTSL::StringView string) {
		auto& primitive = getPrimitive(ui_element_handle);
		textPrimitives[primitive.DerivedTypeIndex].Text = string;
		flagsAsDirty(ui_element_handle);
	}

	void SetFont(const UIElementHandle ui_element_handle, const GTSL::StringView font_name) {
		auto& primitive = getPrimitive(ui_element_handle);
		textPrimitives[primitive.DerivedTypeIndex].Font = font_name;
		flagsAsDirty(ui_element_handle);

		auto* fontResourceManager = GetApplicationManager()->GetSystem<FontResourceManager>(u8"FontResourceManager");
		fontResourceManager->LoadFont(font_name, OnFontLoadTaskHandle);
	}

	UIElementHandle AddCanvas(const GTSL::StringView json_ui_text, const UIElementHandle ui_element_handle = UIElementHandle()) {
		//canvases.Emplace(system);
		auto canvasHandle = add(ui_element_handle, PrimitiveData::PrimitiveType::CANVAS);
		SetScalingPolicy(canvasHandle, 0, ScalingPolicies::FILL);
		SetScalingPolicy(canvasHandle, 1, ScalingPolicies::FILL);
		SetSizingPolicy(canvasHandle, 0, SizingPolicies::FROM_SCREEN);
		SetSizingPolicy(canvasHandle, 1, SizingPolicies::FROM_SCREEN);

		auto processElement = [&](UIElementHandle parent_element_handle, const GTSL::StringView name, GTSL::JSONMember element, auto&& self) -> void {
			if(const auto e = element[u8"enabled"]; e && !e.GetBool()) { return; }

			auto typeString = element[u8"type"].GetStringView();

			PrimitiveData::PrimitiveType type = PrimitiveData::PrimitiveType::NONE;

			switch(GTSL::Hash(typeString)) {
			case GTSL::Hash(u8"Box"): type = PrimitiveData::PrimitiveType::SQUARE; break;
			case GTSL::Hash(u8"Organizer"): type = PrimitiveData::PrimitiveType::ORGANIZER; break;
			case GTSL::Hash(u8"Text"): type = PrimitiveData::PrimitiveType::TEXT; break;
			}

			UIElementHandle elementHandle = add(parent_element_handle, type);

			if(auto sizeL = element[u8"size"]) {
				auto processSize = [&](const uint8 axis, GTSL::JSONMember json_member) {
					if(!json_member) { return; }

					if(auto size = json_member[u8"size"]) {
						SetScale(elementHandle, axis, size.GetFloat());
					}

					if(auto scaling = json_member[u8"scaling"]) {
						ScalingPolicies scalingPolicy = ScalingPolicies::FILL;

						switch(GTSL::Hash(scaling.GetStringView())) {
						case GTSL::Hash(u8"fill"): scalingPolicy = ScalingPolicies::FILL; break;
						case GTSL::Hash(u8"aspect_ratio"): scalingPolicy = ScalingPolicies::SET_ASPECT_RATIO; break;
						}

						SetScalingPolicy(elementHandle, axis, scalingPolicy);
					}

					if(auto sizing = json_member[u8"reference"]) {
						SizingPolicies sizingPolicy = SizingPolicies::FROM_SCREEN;

						switch(GTSL::Hash(sizing.GetStringView())) {
						case GTSL::Hash(u8"parent"): sizingPolicy = SizingPolicies::FROM_PARENT_CONTAINER; break;
						case GTSL::Hash(u8"screen"): sizingPolicy = SizingPolicies::FROM_SCREEN; break;
						}

						SetSizingPolicy(elementHandle, axis, sizingPolicy);
					}
				};

				processSize(0, sizeL[u8"x"]);
				processSize(1, sizeL[u8"y"]);
			}

			if(auto color = element[u8"color"]) {
				SetColor(elementHandle, color.GetStringView());
			}

			if(auto roundness = element[u8"rounding"]) {
				SetRounding(elementHandle, roundness.GetFloat());
			}

			if(auto padding = element[u8"padding"]) {
				SetPadding(elementHandle, padding.GetFloat());
			}

			if(auto spacing = element[u8"spacing"]) {
				SetSpacing(elementHandle, spacing.GetFloat());
			}

			if(auto font = element[u8"font"]) {
				SetFont(elementHandle, font.GetStringView());
			}

			if(auto string = element[u8"string"]) {
				SetString(elementHandle, string.GetStringView());
			}

			if(auto alignmentData = element[u8"alignment"]) {
				Alignments alignment = Alignments::CENTER;

				switch(GTSL::Hash(alignmentData.GetStringView())) {
				case GTSL::Hash(u8"center"): alignment = Alignments::CENTER; break;
				case GTSL::Hash(u8"left"): alignment = Alignments::LEFT; break;
				case GTSL::Hash(u8"right"): alignment = Alignments::RIGHT; break;
				case GTSL::Hash(u8"top"): alignment = Alignments::TOP; break;
				case GTSL::Hash(u8"bottom"): alignment = Alignments::BOTTOM; break;
				}

				SetElementAlignment(elementHandle, alignment);
			}

			if(auto children = element[u8"children"]) {
				for(auto c : children) {
					self(elementHandle, u8"", c, self);
				}
			}
		};

		GTSL::Buffer<BE::TAR> jsonBuffer(GetTransientAllocator());
		GTSL::JSONMember json = GTSL::Parse(json_ui_text, jsonBuffer);

		for(auto c : json[u8"children"]) {
			processElement(canvasHandle, u8"p", c, processElement);
		}


		return canvasHandle;
	}

	UIElementHandle AddOrganizer(const UIElementHandle ui_element_handle = UIElementHandle()) {
		auto organizerHandle = add(ui_element_handle, PrimitiveData::PrimitiveType::ORGANIZER);
		return organizerHandle;
	}

	UIElementHandle AddSquare(const UIElementHandle element_handle = UIElementHandle()) {
		auto handle = add(element_handle, PrimitiveData::PrimitiveType::SQUARE);
		return handle;
	}

	UIElementHandle AddText(const UIElementHandle element_handle, const GTSL::StringView text) {
		auto handle = add(element_handle, PrimitiveData::PrimitiveType::TEXT);
		return GetApplicationManager()->MakeHandle<UIElementHandle>(UIElementTypeIndentifier, handle());
	}

	UIElementHandle AddCurve(const UIElementHandle element_handle) {
		auto handle = add(element_handle, PrimitiveData::PrimitiveType::CURVE);
		const auto place = curvePrimitives.Emplace(GetPersistentAllocator());
		auto& primitive = getPrimitive(handle);
		primitive.DerivedTypeIndex = place;
		auto& curve = curvePrimitives[place];
		return GetApplicationManager()->MakeHandle<UIElementHandle>(UIElementTypeIndentifier, place);
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
		case PrimitiveData::PrimitiveType::NONE: break;
		case PrimitiveData::PrimitiveType::ORGANIZER: break;
		//case PrimitiveData::PrimitiveType::SQUARE: squares[primitive.DerivedTypeIndex].SetColor(color); break;
		}

		auto colorEntry = colors.TryGet(color);

		if(!colorEntry) { return; }

		primitive.Color.X() = colorEntry.Get().R();
		primitive.Color.Y() = colorEntry.Get().G();
		primitive.Color.Z() = colorEntry.Get().B();
		primitive.Color.W() = colorEntry.Get().A();
	}

	void SetMaterial(const UIElementHandle ui_element_handle, const RenderModelHandle material) {
		getPrimitive(ui_element_handle).Material = material;
		flagsAsDirty(ui_element_handle);
	}

	void SetPosition(UIElementHandle ui_element_handle, GTSL::Vector2 pos) {
		auto& primitive = primitives[ui_element_handle()];
		switch (primitive.Type) {
		case PrimitiveData::PrimitiveType::NONE: break;
		case PrimitiveData::PrimitiveType::ORGANIZER: break;
		case PrimitiveData::PrimitiveType::SQUARE: break;
		}
		flagsAsDirty(ui_element_handle);
	}

	void SetElementAlignment(const UIElementHandle organizer, Alignments alignment) {
		getPrimitive(organizer).Alignment = alignment;
		flagsAsDirty(organizer);
	}

	void SetScalingPolicy(UIElementHandle organizer, ScalingPolicies scaling_policy) {
		getPrimitive(organizer).ScalingPolicies[0] = scaling_policy;
		getPrimitive(organizer).ScalingPolicies[1] = scaling_policy;
		flagsAsDirty(organizer);
	}

	void SetScalingPolicy(UIElementHandle organizer, uint8 axis, ScalingPolicies scaling_policy) {
		getPrimitive(organizer).ScalingPolicies[axis] = scaling_policy;
		flagsAsDirty(organizer);
	}

	void SetSizingPolicy(UIElementHandle organizer, SizingPolicies sizing_policy) {
		getPrimitive(organizer).SizingPolicies[0] = sizing_policy;
		getPrimitive(organizer).SizingPolicies[1] = sizing_policy;
		flagsAsDirty(organizer);
	}

	void SetSizingPolicy(UIElementHandle organizer, uint8 axis, SizingPolicies sizing_policy) {
		getPrimitive(organizer).SizingPolicies[axis] = sizing_policy;
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

	GTSL::StringView GetString(const uint32 index) { return textPrimitives[primitives[index].DerivedTypeIndex].Text; }

private:
	GTSL::FixedVector<UIElementHandle, BE::PersistentAllocatorReference> canvases;
	GTSL::HashMap<Id, GTSL::RGBA, BE::PAR> colors;

	GTSL::Tree<PrimitiveData, BE::PAR> primitives;

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
		primitive.HalfSize = 1.0f;
		primitive.SizingPolicies[0] = SizingPolicies::FROM_SCREEN;
		primitive.SizingPolicies[1] = SizingPolicies::FROM_SCREEN;
		primitive.ScalingPolicies[0] = ScalingPolicies::FILL;
		primitive.ScalingPolicies[1] = ScalingPolicies::SET_ASPECT_RATIO;
		primitive.SpacingPolicy = SpacingPolicy::DISTRIBUTE;
		primitive.DerivedTypeIndex = ~0u;
		primitive.isDirty = true;
		primitive.Color = 0.5f;
		primitive.Padding = 0.0f;

		if (parent_handle) {
			flagsAsDirty(parent_handle); //if a child is added to an element it has to be re-evaluated
		}

		switch (type) {
		case PrimitiveData::PrimitiveType::NONE: break;
		case PrimitiveData::PrimitiveType::CANVAS: break;
		case PrimitiveData::PrimitiveType::ORGANIZER: break;
		case PrimitiveData::PrimitiveType::SQUARE: break;
		case PrimitiveData::PrimitiveType::TEXT: {
			primitive.SizingPolicies[0] = SizingPolicies::FROM_PARENT_CONTAINER;
			primitive.SizingPolicies[1] = SizingPolicies::FROM_SCREEN;
			primitive.ScalingPolicies[0] = ScalingPolicies::FILL;
			primitive.ScalingPolicies[1] = ScalingPolicies::AUTO;
			primitive.DerivedTypeIndex = textPrimitives.Emplace(GetPersistentAllocator());
			break;
		}
		case PrimitiveData::PrimitiveType::CURVE: break;
		}

		auto handle = GetApplicationManager()->MakeHandle<UIElementHandle>(UIElementTypeIndentifier, primitiveIndex);

		GetApplicationManager()->DispatchEvent(this, GetOnCreateUIElementEventHandle(), GTSL::MoveRef(handle), GTSL::MoveRef(primitive.Type));

		return handle;
	}

	struct UpdateData {
		GTSL::Vector2 ScreenSize, WindowSize, ScreenToWindowSize, WindowToScreenSize;
	};

	PrimitiveData& updateBranch(GTSL::Tree<PrimitiveData, BE::PersistentAllocatorReference>::iterator iterator, const UpdateData* update_data, GTSL::
	                  Vector2 size, GTSL::Vector2 start_position, GTSL::Vector2 parent_way);

	void flagsAsDirty(const UIElementHandle element_handle) {
		getPrimitive(element_handle).isDirty = true;
	}

	struct FontData {
		FontData(const BE::PAR& allocator) : Characters(128, allocator) {}

		GTSL::Vector<FontResourceManager::Character, BE::PAR> Characters;
	};
	GTSL::HashMap<Id, FontData, BE::PAR> fonts;

	void OnFontLoad(TaskInfo, FontResourceManager::FontData font_data, GTSL::Buffer<BE::PAR> font_buffer);
};

inline void UIManager::OnFontLoad(TaskInfo, FontResourceManager::FontData font_data, GTSL::Buffer<BE::PAR> font_buffer) {
	auto& font = fonts.Emplace(font_data.GetName(), GetPersistentAllocator());

	for(uint32 i = 0; i < font_data.Characters.Length; ++i) {
		font.Characters.EmplaceBack(font_data.Characters.array[i]);
	}
}
