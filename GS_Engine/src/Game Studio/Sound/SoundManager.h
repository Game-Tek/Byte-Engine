#pragma once

#include "SoundMixer.h"

class Sound;

#include "SAPI/AudioDevice.h"

#include "Resources/AudioResourceManager.h"

class SoundManager
{
	SoundMixer* activeSoundMixer = nullptr;

	AudioDevice* audioDevice = nullptr;

	AudioResourceManager::AudioResourceData* sound = nullptr;

	void* buffer = nullptr;
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
