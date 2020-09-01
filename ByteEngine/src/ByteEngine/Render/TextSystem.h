#pragma once

#include <GTSL/StaticString.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vector2.h>

#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/FontResourceManager.h"

class TextSystem : public System
{
public:
	TextSystem() : System("TextSystem") {}

	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	struct AddTextInfo
	{
		GTSL::Vector2 Position;
		GTSL::StaticString<64> Text;
	};
	ComponentReference AddText(const AddTextInfo& addTextInfo);

	const FontResourceManager::Font& GetRenderingFont() const { return renderingFont; }
	
	struct Text
	{
		GTSL::Vector2 Position;
		GTSL::StaticString<64> String;
	};
	GTSL::Ranger<const Text> GetTexts() const { return components; }
private:
	Vector<Text> components;

	FontResourceManager::Font renderingFont;
};
