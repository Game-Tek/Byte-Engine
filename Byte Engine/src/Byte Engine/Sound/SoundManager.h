#pragma once

#include "SoundMixer.h"

class Sound;

#include "Resources/AudioResourceManager.h"

class SoundManager
{
	SoundMixer* activeSoundMixer = nullptr;

	AudioResourceManager::AudioResourceData* sound = nullptr;

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
