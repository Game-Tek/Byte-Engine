#pragma once

#include "SoundMixer.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Game/System.h"

class Sound;

class AudioSystem : public System
{
public:
	AudioSystem();
	~AudioSystem();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown() override;
	
	void PlaySound2D(Sound* _Sound);

	[[nodiscard]] auto GetActiveSoundMixer() const { return activeSoundMixer; }

	template <typename _T>
	void SwapAudioMixer()
	{
		delete activeSoundMixer;
		activeSoundMixer = new _T();
	}

private:
	SoundMixer* activeSoundMixer = nullptr;

	byte* buffer = nullptr;
	uint32 bufferSize = 0;
};
