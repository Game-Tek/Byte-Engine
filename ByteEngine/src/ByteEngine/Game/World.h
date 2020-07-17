#pragma once

#include "ByteEngine/Object.h"

class World : public Object
{
public:
	World();
	~World() = default;

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

protected:
	float worldTimeMultiplier = 1;

};
