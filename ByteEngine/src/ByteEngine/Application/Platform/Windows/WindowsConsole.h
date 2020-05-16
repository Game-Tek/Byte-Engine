#pragma once

#include "ByteEngine/Application/Console.h"

namespace GTSL {
	class String;
}

class WindowsConsole final : public Console
{
	void* inputHandle = nullptr;
	void* outputHandle = nullptr;
	
public:
	WindowsConsole();
	~WindowsConsole();
	void GetLine(GTSL::String& line) override;
	void PutLine(const GTSL::String& line) override;
};
