#pragma once

#include "AudioCore.h"

#if BE_PLATFORM_WINDOWS
#elif BE_PLATFORM_LINUX
#include <alsa/asoundlib.h>
#endif

namespace AAL
{
	/**
	 * \brief Interface for an audio device. Creates and manages an audio device, endpoint and buffer.
	 */
	class AudioDevice
	{
	public:
		AudioDevice() = default;
		~AudioDevice() = default;

		enum class BufferSamplePlacement : GTSL::uint8 { BLOCKS, INTERLEAVED };
		
		struct MixFormat
		{
			GTSL::uint8 NumberOfChannels;
			GTSL::uint32 SamplesPerSecond;
			GTSL::uint8 BitsPerSample;
			
			GTSL::uint8 GetBytesPerSample() const { return BitsPerSample / 8; }
			
			/**
			 * \brief Frame size, in bytes. The frame size is the minimum atomic unit of data for the format.
			 * Frane size is equal to the product of NumberChannels and BitsPerSample divided by 8 (bytes per sample).
			 * Software must process a multiple of BlockAlignment bytes of data at a time. Data written to and read from a device must always start at the beginning of a block.
			 * For example, it is illegal to start playback of PCM data in the middle of a sample (that is, on a non-block-aligned boundary).
			 */
			GTSL::uint16 GetFrameSize() const { return static_cast<GTSL::uint16>(NumberOfChannels) * GetBytesPerSample(); }
		};
		
		struct CreateInfo
		{
		};

		/**
		* \brief Initializes the audio device to start receiving audio. Must be called before any other function.
		*/
		[[nodiscard]] bool Initialize(const CreateInfo&) {
#if BE_PLATFORM_WINDOWS
			if(CoInitializeEx(nullptr, 0) != S_OK) { return false; }

			if(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL, __uuidof(IMMDeviceEnumerator), reinterpret_cast<void**>(&enumerator)) != S_OK) { return false; }

			if(enumerator->GetDefaultAudioEndpoint(eRender, eConsole, &endPoint) != S_OK) { return false; }

			if(endPoint->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr, reinterpret_cast<void**>(&audioClient)) != S_OK) { return false; }

			return true;
#elif BE_PLATFORM_LINUX
			if(snd_pcm_open(&device, "default", SND_PCM_STREAM_PLAYBACK, 0) < 0) { return false; }
			snd_pcm_hw_params_alloca(&hwParams);

			return true;
#endif
		}

		/**
		 * \brief Return the optimal mix format supported by the audio device. Must be called after initialize.
		 * \return MixFormat supported by the audio device.
		 */
		[[nodiscard]] GTSL::Result<MixFormat> GetMixFormat() const {
#if BE_PLATFORM_WINDOWS
			WAVEFORMATEXTENSIBLE* waveformatex;
			
			if(audioClient->GetMixFormat(reinterpret_cast<WAVEFORMATEX**>(&waveformatex)) != S_OK) {
				return GTSL::Result<MixFormat>(false);
			}

			MixFormat mixFormat;

			GTSL_ASSERT(waveformatex->Format.wFormatTag == WAVE_FORMAT_PCM, "Format mismatch!");

			mixFormat.NumberOfChannels = static_cast<GTSL::uint8>(waveformatex->Format.nChannels);
			mixFormat.SamplesPerSecond = waveformatex->Format.nSamplesPerSec;
			mixFormat.BitsPerSample = static_cast<GTSL::uint8>(waveformatex->Format.wBitsPerSample != 24 ? waveformatex->Format.wBitsPerSample : 32); // Most devices using WASAPI prefer 24 bits padded to 32 bits per sample.

			CoTaskMemFree(waveformatex);

			return GTSL::Result(GTSL::MoveRef(mixFormat), true);
#elif BE_PLATFORM_LINUX
			snd_pcm_hw_params_any(device, hwParams);
			int bits = snd_pcm_hw_params_get_sbits(hwParams);
			uint32 sampleRate = 0;
			snd_pcm_hw_params_get_rate(hwParams, &sampleRate, nullptr);
			uint32 channels = 0;
			snd_pcm_hw_params_get_channels(hwParams, &channels);
			return GTSL::Result(MixFormat{ channels, sampleRate, bits }, true);
#endif
		}

		BufferSamplePlacement GetBufferSamplePlacement() const { return BufferSamplePlacement::BLOCKS; }

		/**
		 * \brief Queries the audio device for support of the specified format with the specified share mode.
		 * \param shareMode Shared mode to query support for.
		 * \param mixFormat Mix format to check support for.
		 * \return Wheter the format is supported(true) or not(false).
		 */
		[[nodiscard]] bool IsMixFormatSupported(StreamShareMode shareMode, MixFormat mixFormat) const {
#if BE_PLATFORM_WINDOWS
			WAVEFORMATEX waveformatex; WAVEFORMATEXTENSIBLE* closestMatch;

			waveformatex.wFormatTag = WAVE_FORMAT_PCM;
			waveformatex.cbSize = 0; //extra data size if using WAVEFORMATEXTENSIBLE, this parameter is ignored since format is PCM but for correctness we set it to 0
			waveformatex.nBlockAlign = mixFormat.GetFrameSize();
			waveformatex.nChannels = mixFormat.NumberOfChannels;
			waveformatex.nSamplesPerSec = mixFormat.SamplesPerSecond;
			waveformatex.wBitsPerSample = mixFormat.BitsPerSample;
			waveformatex.nAvgBytesPerSec = waveformatex.nSamplesPerSec * waveformatex.nBlockAlign;

			bool result = false;

			switch (shareMode)
			{
			case StreamShareMode::SHARED:
				result = audioClient->IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, &waveformatex, reinterpret_cast<WAVEFORMATEX**>(&closestMatch)) == S_OK;
				CoTaskMemFree(closestMatch);
				break;

			case StreamShareMode::EXCLUSIVE:
				result = audioClient->IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &waveformatex, nullptr) == S_OK;
				break;
			}

			return result;
#elif BE_PLATFORM_LINUX
			return true;
#endif
		}

		/**
		 * \brief Creates the audio stream with the requested parameters. The user must make sure the utilized parameter combination is supported by querying the suport function.
		 * \param shareMode Share mode to initialize the audio stream with.
		 * \param mixFormat Mix format to initialize the audio stream with.
		 */
		bool CreateAudioStream(StreamShareMode shareMode, MixFormat mixFormat) {
#if BE_PLATFORM_WINDOWS
			WAVEFORMATEXTENSIBLE pwfx{};
			pwfx.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
			pwfx.Format.cbSize = 22;
			pwfx.Format.nChannels = 2;
			pwfx.Format.nSamplesPerSec = mixFormat.SamplesPerSecond;
			pwfx.Format.wBitsPerSample = pwfx.Samples.wValidBitsPerSample = mixFormat.BitsPerSample;
			mixFormat.BitsPerSample = static_cast<GTSL::uint8>(mixFormat.BitsPerSample != 24 ? mixFormat.BitsPerSample : 32); // Most devices using WASAPI prefer 24 bits padded to 32 bits per sample.
			pwfx.Format.nBlockAlign = mixFormat.GetFrameSize();
			pwfx.SubFormat = GUID{ STATIC_KSDATAFORMAT_SUBTYPE_PCM };
			pwfx.Format.nAvgBytesPerSec = mixFormat.SamplesPerSecond * pwfx.Format.nBlockAlign;

			frameSize = pwfx.Format.nBlockAlign;

			_AUDCLNT_SHAREMODE win_share_mode{};
			switch (shareMode) {
			case StreamShareMode::EXCLUSIVE: win_share_mode = AUDCLNT_SHAREMODE_EXCLUSIVE; break;
			case StreamShareMode::SHARED: win_share_mode = AUDCLNT_SHAREMODE_SHARED; break;
			}
			
			if(audioClient->Initialize(win_share_mode, 0, 0, 0, reinterpret_cast<PWAVEFORMATEX>(&pwfx), nullptr) != S_OK) {
				return false;
			}

			if(audioClient->GetBufferSize(&bufferFrameCount) != S_OK) {
				return false;
			}

			if(audioClient->GetService(__uuidof(IAudioRenderClient), reinterpret_cast<void**>(&renderClient)) != S_OK) {
				return false;
			}

			return true;
#elif BE_PLATFORM_LINUX
			uint32 resample = 1;
			snd_pcm_hw_params_set_rate_resample(device, hwParams, resample);

			if(snd_pcm_hw_params_set_access(device, hwParams, SND_PCM_ACCESS_RW_INTERLEAVED) < 0) { return false; }

			snd_pcm_format_t format = SND_PCM_FORMAT_S16_LE;

			switch (mixFormat.BitsPerSample) {
				case 8: format = SND_PCM_FORMAT_U8; break;
				case 16: format = SND_PCM_FORMAT_S16_LE; break;
				case 24: format = SND_PCM_FORMAT_S24_LE; break;
				case 32: format = SND_PCM_FORMAT_S32_LE; break;
			}

			if(snd_pcm_hw_params_set_format(device, hwParams, format) < 0) { return false; }

			if(snd_pcm_hw_params_set_channels(device, hwParams, mixFormat.NumberOfChannels) < 0) { return false; }

			uint32 exactRate = mixFormat.SamplesPerSecond;
			if(snd_pcm_hw_params_set_rate_near(device, hwParams, &exactRate, nullptr) < 0) { return false; }

			snd_pcm_hw_params(device, hwParams); // Write the parameters to the driver

			return true;
#endif
		}

		/**
		 * \brief Starts the audio stream. No samples can be pushed if the stream is not started.
		 */
		[[nodiscard]] bool Start() const {
#if BE_PLATFORM_WINDOWS
			if(audioClient->Start() != S_OK) {
				return false;
			}

			return true;
#elif BE_PLATFORM_LINUX
			snd_pcm_prepare(device);
			return true;
#endif
		}

		/**
		* \brief Sets the passed variable as the available size in the allocated buffer.
		* Should be called to query the available size before filling the audio buffer size it may have some space still occupied since the audio driver may not have consumed it.
		* \param availableBufferFrames Pointer to a variable to set as the available buffer size.
		*/
		bool GetAvailableBufferFrames(GTSL::uint32& availableBufferFrames) const {
#if BE_PLATFORM_WINDOWS
			UINT32 numFramesAvailable = 0;
			//For a shared-mode rendering stream, the padding value reported by GetCurrentPadding specifies the number of audio frames
			//that are queued up to play in the endpoint buffer. Before writing to the endpoint buffer, the client can calculate the
			//amount of available space in the buffer by subtracting the padding value from the buffer length.
			if(audioClient->GetCurrentPadding(&numFramesAvailable) != S_OK) {
				return false;
			}

			availableBufferFrames = bufferFrameCount - numFramesAvailable;

			return true;
#elif BE_PLATFORM_LINUX
			return true;
#endif
		}

		/**
		* \brief Sets the passed variable as the size of the allocated buffer.
		* \param totalBufferFrames Pointer to to variable for storing the size of the allocated buffer.
		*/
		void GetBufferFrameCount(GTSL::uint32& totalBufferFrames) const {
#if BE_PLATFORM_WINDOWS
			audioClient->GetBufferSize(&totalBufferFrames);
#elif BE_PLATFORM_LINUX
			snd_pcm_uframes_t frames;
			snd_pcm_hw_params_get_buffer_size(hwParams, &frames);
			totalBufferFrames = static_cast<GTSL::uint32>(frames);
#endif
		}

		/**
		* \brief Invokes a function to push audio data for the amount of specified samples to the audio device buffer, making such data available for the next request from the driver to retrieve audio data.
		* \param copyFunction Callable object taking a uint32 specifying the size in bytes to copy, and a void* to copy the data to.
		* \param pushedSamples Number of audio frames to copy to the audio buffer.
		*/
		template<typename F>
		bool PushAudioData(F&& copyFunction, GTSL::uint32 pushedSamples) const
		{
#if BE_PLATFORM_WINDOWS
			auto getResult = getBuffer(pushedSamples);
			if(!getResult) { return false; }
			copyFunction(pushedSamples * frameSize, getResult.Get());
			auto releaseResult = releaseBuffer(pushedSamples);
			if (!releaseResult) { return false; }
			return true;
#elif BE_PLATFORM_LINUX
			return true;
#endif
		}

		/**
		 * \brief Stops the audio stream. No samples can be pushed if the stream is not started. Must be called before destroying the audio device, no other functions shall be called after this.
		*/
		[[nodiscard]] bool Stop() const {
#if BE_PLATFORM_WINDOWS
			if(audioClient->Stop() != S_OK) {
				return false;
			}

			return true;
#elif BE_PLATFORM_LINUX
			return true;
#endif
		}

		/**
		 * \brief Destroys the audio stream. Must be called before destroying the audio device, no other functions shall be called after this.
		 */
		void Destroy() {
#if BE_PLATFORM_WINDOWS
			renderClient->Release();
			audioClient->Release();
			endPoint->Release();
			enumerator->Release();

			CoUninitialize();
#elif BE_PLATFORM_LINUX
			snd_pcm_hw_params_free(hwParams);
			snd_pcm_close(device);
#endif
		}

		static constexpr GTSL::uint8 LEFT_CHANNEL = 0, RIGHT_CHANNEL = 1;

	private:
#if BE_PLATFORM_WINDOWS
		/**
		 * \brief The IMMDeviceEnumerator interface provides methods for enumerating multimedia device resources.
		 * In the current implementation of the MMDevice API, the only device resources that this interface can enumerate are audio endpoint devices.
		 * A client obtains a reference to an IMMDeviceEnumerator interface by calling the CoCreateInstance function, as described previously (see MMDevice API).
		 */
		IMMDeviceEnumerator* enumerator = nullptr;
		
		/**
		 * \brief The IMMDevice interface encapsulates the generic features of a multimedia device resource.
		 * In the current implementation of the MMDevice API, the only type of device resource that an IMMDevice interface can represent is an audio endpoint device.
		 */
		IMMDevice* endPoint = nullptr;
		
		/**
		 * \brief The IAudioClient interface enables a client to create and initialize an audio stream between an audio application and the audio engine
		 * (for a shared-mode stream) or the hardware buffer of an audio endpoint device (for an exclusive-mode stream).
		 */
		IAudioClient* audioClient = nullptr;
		
		/**
		 * \brief The IAudioRenderClient interface enables a client to write output data to a rendering endpoint buffer.
		 * The client obtains a reference to the IAudioRenderClient interface of a stream object by calling the IAudioClient::GetService method
		 * with parameter riid set to REFIID IID_IAudioRenderClient.
		 */
		IAudioRenderClient* renderClient = nullptr;

		GTSL::uint32 frameSize = 0;

		GTSL::uint32 bufferFrameCount = 0;

		[[nodiscard]] GTSL::Result<void*> getBuffer(GTSL::uint32 pushedSamples) const {
			BYTE* bufferAddress = nullptr;
			auto result = renderClient->GetBuffer(pushedSamples, &bufferAddress);
			return GTSL::Result(GTSL::MoveRef(static_cast<void*>(bufferAddress)), result == S_OK);
		}

		[[nodiscard]] bool releaseBuffer(GTSL::uint32 pushedSamples) const {
			return renderClient->ReleaseBuffer(pushedSamples, 0) == S_OK;
		}
#elif BE_PLATFORM_LINUX
		snd_pcm_t* device = nullptr;
		snd_pcm_hw_params_t* hwParams = nullptr;
#endif
	};
}