#pragma once

#include "Object.h"

#include "Application/Application.h"

class Texture : public Object
{
	ResourceReference textureResource;
public:

	explicit Texture(const GTSL::String& name) : textureResource(BE::Application::Get()->GetResourceManager()->TryGetResource(name, "Texture"))
	{
	}

	~Texture()
	{
		BE::Application::Get()->GetResourceManager()->ReleaseResource(textureResource);
	}
	
	[[nodiscard]] const char* GetName() const override { return "Texture"; }
};
