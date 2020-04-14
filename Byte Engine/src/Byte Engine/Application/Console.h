#pragma once

namespace GTSL {
	class String;
}

class Console
{
public:
	Console() = default;
	virtual ~Console() = default;
	
	virtual void GetLine(GTSL::String& line) = 0;
	virtual void PutLine(const GTSL::String& line) = 0;
};
