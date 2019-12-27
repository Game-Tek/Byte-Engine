#pragma once

#include "Object.h"

#include "Resources/TextureResource.h"
#include "Application/Application.h"

class Texture : public Object
{
	TextureResource* textureResource = nullptr;
public:
	
	explicit Texture(const FString& name) : textureResource(GS::Application::Get()->GetResourceManager()->GetResource<TextureResource>(name))
	{
		
	}

	~Texture()
	{
		GS::Application::Get()->GetResourceManager()->ReleaseResource(textureResource);
	}

	[[nodiscard]] const char* GetName() const override { return "Texture"; }
};
