#pragma once

#include "SoundMixer.h"
#include "ByteEngine/Core.h"

class Sound;

class SoundManager
{
	SoundMixer* activeSoundMixer = nullptr;

	byte* buffer = nullptr;
	uint32 bufferSize = 0;
public:
	SoundManager();
	~SoundManager();
	
	void Update();
	
	void PlaySound2D(Sound* _Sound);

	[[nodiscard]] auto GetActiveSoundMixer() const { return activeSoundMixer; }

	template <typename _T>
	void SwapAudioMixer()
	{
		delete activeSoundMixer;
		activeSoundMixer = new _T();
	}
};
