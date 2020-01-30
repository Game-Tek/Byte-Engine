#include "WindowsAudioDevice.h"

#include <mmdeviceapi.h>
#include <exception>

#define SAFE_RELEASE(punk)  \
              if ((punk) != NULL)  \
                { (punk)->Release(); (punk) = NULL; }


WindowsAudioDevice::WindowsAudioDevice()
{
	const auto CLSID_MMDeviceEnumerator = __uuidof(MMDeviceEnumerator);
	const auto IID_IMMDeviceEnumerator = __uuidof(IMMDeviceEnumerator);
	const auto IID_IAudioClient = __uuidof(IAudioClient);
	const auto IID_IAudioRenderClient = __uuidof(IAudioRenderClient);

	HRESULT hr = CoCreateInstance(CLSID_MMDeviceEnumerator, NULL, CLSCTX_ALL, IID_IMMDeviceEnumerator,
	                              reinterpret_cast<void**>(&enumerator));

	//IMMDeviceCollection* audio_endpoints = nullptr;
	//enumerator->EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE, &audio_endpoints);

	enumerator->GetDefaultAudioEndpoint(eRender, eConsole, &endPoint);

	endPoint->Activate(IID_IAudioClient, CLSCTX_ALL, NULL, reinterpret_cast<void**>(&audioClient));

	audioClient->Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, pwfx, NULL);

	audioClient->GetService(IID_IAudioRenderClient, reinterpret_cast<void**>(&renderClient));

	audioClient->GetBufferSize(&bufferFrameCount);
	
	renderClient->GetBuffer(bufferFrameCount, reinterpret_cast<BYTE**>(&data));

	switch (pwfx->nChannels)
	{
	case 1: channelCount = AudioChannelCount::CHANNELS_MONO; break;
	case 2: channelCount = AudioChannelCount::CHANNELS_STEREO; break;
	case 6: channelCount = AudioChannelCount::CHANNELS_5_1; break;
	case 8: channelCount = AudioChannelCount::CHANNELS_7_1; break;
	default: throw std::exception("Channel count not supported!");
	}

	switch (pwfx->nSamplesPerSec)
	{
	case 44100: sampleRate = AudioSampleRate::KHZ_44_1; break;
	case 48000: sampleRate = AudioSampleRate::KHZ_48; break;
	case 96000: sampleRate = AudioSampleRate::KHZ_96; break;
	default: throw std::exception("Sample rate not supported!");
	}

	switch (pwfx->wBitsPerSample)
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

void WindowsAudioDevice::Start()
{
	audioClient->Start();
}

void WindowsAudioDevice::GetAvailableBufferSize(uint64* available_buffer_size_)
{
	UINT32 numFramesPadding = 0;
	audioClient->GetCurrentPadding(&numFramesPadding);

	*available_buffer_size_ = bufferFrameCount - numFramesPadding;
}

void WindowsAudioDevice::GetBufferSize(uint32* total_buffer_size_)
{
	audioClient->GetBufferSize(total_buffer_size_);
}

void WindowsAudioDevice::PushAudioData(void* data_, uint64 pushed_samples_)
{
	UINT32 numFramesPadding = 0;
	audioClient->GetCurrentPadding(&numFramesPadding);

	const auto abs = bufferFrameCount - numFramesPadding;

	BYTE* data = nullptr;
	renderClient->GetBuffer(static_cast<uint32>(abs), &data);
	memcpy(data, data_, pwfx->nBlockAlign);
	renderClient->ReleaseBuffer(abs, 0);
}

void WindowsAudioDevice::Stop()
{
	audioClient->Stop();
}
