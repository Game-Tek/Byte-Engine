#pragma once

#include "ByteEngine/Object.h"

class World : public Object
{
public:
	World();
	~World() = default;

	struct InitializeInfo
	{
		class ApplicationManager* GameInstance{ nullptr };
	};
	virtual void InitializeWorld(const InitializeInfo& initializeInfo);

	struct DestroyInfo
	{
		class ApplicationManager* GameInstance{ nullptr };
	};
	virtual void DestroyWorld(const DestroyInfo& destroyInfo);
	
	virtual void Pause();

	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }

protected:
	float worldTimeMultiplier = 1;

};
