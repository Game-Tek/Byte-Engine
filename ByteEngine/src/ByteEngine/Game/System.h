#pragma once

#include "ByteEngine/Object.h"

/**
 * \brief Systems persist across levels and can process world components regardless of the current level.
 * Used to instantiate render engines, sound engines, physics engines, AI systems, etc.
 */
class System : public Object
{
public:
	using ComponentReference = uint32;

	struct InitializeInfo
	{
		class GameInstance* GameInstance{ nullptr };
	};
	virtual void Initialize(const InitializeInfo& initializeInfo) = 0;

	virtual void Shutdown() = 0;

	[[nodiscard]] const char* GetName() const override { return "System"; }
private:
};
