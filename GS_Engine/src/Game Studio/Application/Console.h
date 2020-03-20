#pragma once

class FString;

class Console
{
public:
	Console() = default;
	virtual ~Console() = default;
	
	virtual void GetLine(FString& line) = 0;
	virtual void PutLine(const FString& line) = 0;
};
