#include "WindowsAudioDevice.h"

#include <mmdeviceapi.h>
#include <exception>

#define SAFE_RELEASE(punk)  \
              if ((punk) != NULL)  \
                { (punk)->Release(); (punk) = NULL; }


WindowsAudioDevice::WindowsAudioDevice(const AudioDeviceCreateInfo& audioDeviceCreateInfo)
{
	const auto CLSID_MMDeviceEnumerator = __uuidof(MMDeviceEnumerator);
	const auto IID_IMMDeviceEnumerator = __uuidof(IMMDeviceEnumerator);
	const auto IID_IAudioClient = __uuidof(IAudioClient);
	const auto IID_IAudioRenderClient = __uuidof(IAudioRenderClient);

	HRESULT hr = CoCreateInstance(CLSID_MMDeviceEnumerator, NULL, CLSCTX_ALL, IID_IMMDeviceEnumerator, reinterpret_cast<void**>(&enumerator));

	//IMMDeviceCollection* audio_endpoints = nullptr;
	//enumerator->EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE, &audio_endpoints);

	enumerator->GetDefaultAudioEndpoint(eRender, eConsole, &endPoint);

	endPoint->Activate(IID_IAudioClient, CLSCTX_ALL, NULL, reinterpret_cast<void**>(&audioClient));

	_AUDCLNT_SHAREMODE win_share_mode{};
	switch (audioDeviceCreateInfo.ShareMode)
	{
	case StreamShareMode::EXCLUSIVE: win_share_mode = _AUDCLNT_SHAREMODE::AUDCLNT_SHAREMODE_EXCLUSIVE; break;
	case StreamShareMode::SHARED: win_share_mode = _AUDCLNT_SHAREMODE::AUDCLNT_SHAREMODE_SHARED; break;
	}
	audioClient->Initialize(win_share_mode, 0, 0, 0, &pwfx->Format, nullptr);

	audioClient->GetService(IID_IAudioRenderClient, reinterpret_cast<void**>(&renderClient));

	audioClient->GetBufferSize(&bufferFrameCount);
	
	renderClient->GetBuffer(bufferFrameCount, reinterpret_cast<BYTE**>(&data));

	switch (pwfx->Format.nChannels)
	{
	case 1: channelCount = AudioChannelCount::CHANNELS_MONO; break;
	case 2: channelCount = AudioChannelCount::CHANNELS_STEREO; break;
	case 6: channelCount = AudioChannelCount::CHANNELS_5_1; break;
	case 8: channelCount = AudioChannelCount::CHANNELS_7_1; break;
	default: throw std::exception("Channel count not supported!");
	}

	switch (pwfx->Format.nSamplesPerSec)
	{
	case 44100: sampleRate = AudioSampleRate::KHZ_44_1; break;
	case 48000: sampleRate = AudioSampleRate::KHZ_48; break;
	case 96000: sampleRate = AudioSampleRate::KHZ_96; break;
	default: throw std::exception("Sample rate not supported!");
	}

	switch (pwfx->Format.wBitsPerSample)
	{
	case 8: bitDepth = AudioBitDepth::BIT_DEPTH_8; break;
	case 16:bitDepth = AudioBitDepth::BIT_DEPTH_16; break;
	case 24:bitDepth = AudioBitDepth::BIT_DEPTH_24; break;
	default: throw std::exception("Bit-depth not supported!");
	}
}

WindowsAudioDevice::~WindowsAudioDevice()
{
	CoTaskMemFree(pwfx);
	SAFE_RELEASE(renderClient)
	SAFE_RELEASE(audioClient)
	SAFE_RELEASE(endPoint)
	SAFE_RELEASE(enumerator)
}

void WindowsAudioDevice::Start() { audioClient->Start(); }

void WindowsAudioDevice::GetAvailableBufferSize(uint64* availableBufferSize)
{
	UINT32 numFramesPadding = 0;
	audioClient->GetCurrentPadding(&numFramesPadding);

	*availableBufferSize = bufferFrameCount - numFramesPadding;
}

void WindowsAudioDevice::GetBufferSize(uint32* totalBufferSize)
{
	audioClient->GetBufferSize(totalBufferSize);
}

void WindowsAudioDevice::PushAudioData(void* data, uint64 pushedSamples)
{
	UINT32 numFramesPadding = 0;
	audioClient->GetCurrentPadding(&numFramesPadding);

	const auto abs = bufferFrameCount - numFramesPadding;

	BYTE* buffer_address = nullptr;
	renderClient->GetBuffer(static_cast<uint32>(abs), &buffer_address);
	memcpy(buffer_address, data, pwfx->Format.nBlockAlign);
	renderClient->ReleaseBuffer(abs, 0);
}

void WindowsAudioDevice::Stop() { audioClient->Stop(); }
