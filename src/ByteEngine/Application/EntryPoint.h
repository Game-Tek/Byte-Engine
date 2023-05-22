#pragma once

#include "Application.h"
#include <GTSL/String.hpp>

//extern int CreateApplication(GTSL::Range<const GTSL::StringView*> arguments);
extern int CreateApplication() ;

inline bool BasicCompatibilityTest()
{
	return sizeof(char8_t) == 1 && sizeof(GTSL::uint8) == 1 && sizeof(GTSL::uint16) == 2 && sizeof(GTSL::uint32) == 4 && sizeof(GTSL::uint64) == 8;
}

#ifndef CUSTOM_MAIN
int main(int argc,char** argv)
{
	if (!BasicCompatibilityTest()) return -1;

	return CreateApplication();
	/*GTSL::StringView arguments[32];

	for (auto i = 0; i < argc; ++i)
		arguments[i] = GTSL::StringView(argv[i]);


	return CreateApplication({argc,argv});*/
}
#endif 