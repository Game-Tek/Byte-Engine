#include "WindowsAudioDevice.h"

#include <mmdeviceapi.h>

#define SAFE_RELEASE(punk)  \
              if ((punk) != NULL)  \
                { (punk)->Release(); (punk) = NULL; }


WindowsAudioDevice::WindowsAudioDevice()
{
	const CLSID CLSID_MMDeviceEnumerator = __uuidof(MMDeviceEnumerator);
	const IID IID_IMMDeviceEnumerator = __uuidof(IMMDeviceEnumerator);
	const IID IID_IAudioClient = __uuidof(IAudioClient);
	const IID IID_IAudioRenderClient = __uuidof(IAudioRenderClient);

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
