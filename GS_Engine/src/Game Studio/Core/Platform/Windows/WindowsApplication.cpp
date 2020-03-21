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
		if(XInputGetState(i, &states[i]) != ERROR_SUCCESS)
		{
			--connectedControllers;
			onControllerDisconnected(i);
			break;
		}
		
		if (states[i].Gamepad.bLeftTrigger != input_states[i].Gamepad.bLeftTrigger)
		{
			onLeftTriggerChanged(states[i].Gamepad.bLeftTrigger / 255.0f, (input_states[i].Gamepad.bLeftTrigger - states[i].Gamepad.bLeftTrigger) / 255.0f);
		}
		
		if (states[i].Gamepad.bRightTrigger != input_states[i].Gamepad.bRightTrigger)
		{
			onRightTriggerChanged(states[i].Gamepad.bRightTrigger / 255.0f, (input_states[i].Gamepad.bRightTrigger - states[i].Gamepad.bRightTrigger) / 255.0f);
		}
		
		if (states[i].Gamepad.sThumbLX != input_states[i].Gamepad.sThumbLX || states[i].Gamepad.sThumbLY != input_states[i].Gamepad.sThumbLY)
		{
			onLeftStickMove({ states[i].Gamepad.sThumbLX / 32767.f, states[i].Gamepad.sThumbLY / 32767.f }, { (states[i].Gamepad.sThumbLX - input_states[i].Gamepad.sThumbLX) / 32767.f, (states[i].Gamepad.sThumbLY - input_states[i].Gamepad.sThumbLY) / 32767.f });
		}
		
		if (states[i].Gamepad.sThumbRX != input_states[i].Gamepad.sThumbRX  || states[i].Gamepad.sThumbRY != input_states[i].Gamepad.sThumbRY)
		{
			onRightStickMove({ states[i].Gamepad.sThumbRX / 32767.f, states[i].Gamepad.sThumbRY / 32767.f }, { (states[i].Gamepad.sThumbRX - input_states[i].Gamepad.sThumbRX) / 32767.f, (states[i].Gamepad.sThumbRY - input_states[i].Gamepad.sThumbRY) / 32767.f });
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_UP) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_UP))
		{
			onTopDPadButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_UP));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_DOWN) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_DOWN))
		{
			onBottomDPadButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_DOWN));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_LEFT) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_LEFT))
		{
			onLeftDPadButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_LEFT));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT))
		{
			onRightDPadButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_START) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_START))
		{
			onStartButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_START));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_BACK) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_BACK))
		{
			onBackButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_BACK));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_THUMB) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_THUMB))
		{
			onLeftStickButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_THUMB));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB))
		{
			onRightStickButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER))
		{
			onLeftHatChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER))
		{
			onRightHatChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_A) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_A))
		{
			onBottomFaceButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_A));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_B) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_B))
		{
			onRightFaceButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_B));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_X) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_X))
		{
			onLeftFaceButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_X));
		}
		
		if ((states[i].Gamepad.wButtons & XINPUT_GAMEPAD_Y) != (input_states[i].Gamepad.wButtons & XINPUT_GAMEPAD_Y))
		{
			onTopFaceButtonChanged(intToGamepadButtonState(states[i].Gamepad.wButtons & XINPUT_GAMEPAD_Y));
		}

		input_states[i] = states[i];
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
		if(XInputGetState(i, &state) == ERROR_SUCCESS)
		{
			if (i > connectedControllers)
			{
				onControllerConnected(i);
				++connectedControllers;
			}
		}
		else
		{
			if(i < connectedControllers)
			{
				onControllerDisconnected(i);
				--connectedControllers;
			}
		}
	}
}
