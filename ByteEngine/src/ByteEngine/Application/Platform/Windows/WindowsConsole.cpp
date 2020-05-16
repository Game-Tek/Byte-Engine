#include "WindowsConsole.h"

#include <GTSL/String.hpp>

#include "Windows.h"

WindowsConsole::WindowsConsole() : Console(), inputHandle(GetStdHandle(STD_INPUT_HANDLE)), outputHandle(GetStdHandle(STD_OUTPUT_HANDLE))
{
}

WindowsConsole::~WindowsConsole()
{
}

void WindowsConsole::GetLine(GTSL::String& line)
{
	char buffer[255];
	unsigned long chars_read = 0;
	ReadConsoleA(inputHandle, buffer, 255, &chars_read, NULL);
	line.Insert(buffer, 0);
	line.Drop(chars_read - 2);
}

void WindowsConsole::PutLine(const GTSL::String& line)
{
	unsigned long chars_read = 0;
	WriteConsoleA(outputHandle, line.c_str(), line.GetLength(), &chars_read, nullptr);
}
