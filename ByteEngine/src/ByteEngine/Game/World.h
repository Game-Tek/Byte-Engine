#pragma once

#include "ByteEngine/Object.h"

class World : public Object
{
	float worldTimeMultiplier = 1;

public:
	World();
	virtual ~World() = default;

	[[nodiscard]] const char* GetName() const override { return "World"; }

	struct InitializeInfo
	{
		class GameInstance* GameInstance{ nullptr };
	};
	virtual void InitializeWorld(const InitializeInfo& initializeInfo);

	struct DestroyInfo
	{
		class GameInstance* GameInstance{ nullptr };
	};
	virtual void DestroyWorld(const DestroyInfo& destroyInfo);
	
	virtual void Pause();

	void SetWorldTimeMultiplier(const float multiplier) { worldTimeMultiplier = multiplier; }
};
