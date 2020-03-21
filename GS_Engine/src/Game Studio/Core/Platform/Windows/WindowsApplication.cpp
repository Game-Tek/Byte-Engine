#include "WindowsApplication.h"

WindowsApplication::WindowsApplication(const ApplicationCreateInfo& applicationCreateInfo) : nApplication(applicationCreateInfo), instance(GetModuleHandle(nullptr))
{
}

void WindowsApplication::Update()
{
	MSG message;
	GetMessageA(&message, nullptr, 0, 0);
	TranslateMessage(&message);
	DispatchMessageA(&message);

	XINPUT_STATE states[XUSER_MAX_COUNT];
	for(uint8 i = 0; i < connectedControllers; ++i)
	{
		XInputGetState(i, &states[i]);
		
		if (states[i].Gamepad.bLeftTrigger != input_states[i].Gamepad.bLeftTrigger);
		if (states[i].Gamepad.bRightTrigger != input_states[i].Gamepad.bRightTrigger);
		if (states[i].Gamepad.sThumbLX != input_states[i].Gamepad.sThumbLX);
		if (states[i].Gamepad.sThumbLY != input_states[i].Gamepad.sThumbLY);
		if (states[i].Gamepad.sThumbRX != input_states[i].Gamepad.sThumbRX);
		if (states[i].Gamepad.sThumbRY != input_states[i].Gamepad.sThumbRY);
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_UP) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_UP));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_DOWN) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_DOWN));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_LEFT) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_LEFT));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_START) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_START));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_BACK) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_BACK));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_THUMB) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_THUMB));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_A) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_A));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_B) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_B));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_X) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_X));
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_Y) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_Y));
	}
}

void WindowsApplication::Close()
{
	PostQuitMessage(0);
}

void WindowsApplication::CheckForNewControllers()
{
	XINPUT_STATE state;
	for (uint8 i = 0; i < XUSER_MAX_COUNT; ++i)
	{
		if(XInputGetState(i, &state) == ERROR_SUCCESS) { ++connectedControllers; }	
	}
}
