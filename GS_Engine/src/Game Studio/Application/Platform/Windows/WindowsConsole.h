#pragma once

#include "Application/Console.h"

class WindowsConsole final : public Console
{
	void* inputHandle = nullptr;
	void* outputHandle = nullptr;
	
public:
	WindowsConsole();
	~WindowsConsole();
	void GetLine(FString& line) override;
	void PutLine(const FString& line) override;
};
