#pragma once

#include "SoundMixer.h"

class Sound;

class SoundManager
{
	SoundMixer* activeSoundMixer = nullptr;
public:
	void PlaySound2D(Sound* _Sound);

	[[nodiscard]] auto GetActiveSoundMixer() const { return activeSoundMixer; }

	template <typename _T>
	void SwapAudioMixer()
	{
		delete activeSoundMixer;
		activeSoundMixer = new _T();
	}
};
