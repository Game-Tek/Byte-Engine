#pragma once

#include "ByteEngine/Game/System.h"

class TestSystem : public System
{
public:

	void Initialize(const InitializeInfo& initializeInfo) override {}
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
private:
};
