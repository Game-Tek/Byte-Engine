#pragma once

#include "Object.h"

#include "Application/Application.h"

class Texture : public Object
{
	ResourceReference textureResource;
public:

	explicit Texture(const FString& name) : textureResource(GS::Application::Get()->GetResourceManager()->TryGetResource(name, "Texture"))
	{
	}

	~Texture()
	{
		GS::Application::Get()->GetResourceManager()->ReleaseResource(textureResource);
	}
	
	[[nodiscard]] const char* GetName() const override { return "Texture"; }
};
