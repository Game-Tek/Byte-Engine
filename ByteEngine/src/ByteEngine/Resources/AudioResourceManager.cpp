#include "AudioResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>

#include "ByteEngine/Debug/Assert.h"

#include "ByteEngine/Application/Application.h"

AudioResourceManager::AudioResourceManager() : ResourceManager(u8"AudioResourceManager"), audioResourceInfos(8, 0.25, GetPersistentAllocator()), audioBytes(8, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, package_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication(); query_path += u8"/resources/"; query_path += u8"*.wav";
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += u8"/resources/";
	index_path += BE::Application::Get()->GetPathToApplication(); index_path += u8"/resources/Audio.beidx";
	package_path += BE::Application::Get()->GetPathToApplication(); package_path += u8"/resources/Audio.bepkg";

	indexFile.Open(index_path, GTSL::File::WRITE | GTSL::File::READ, true);
	
	GTSL::Buffer<BE::TAR> file_buffer; file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());
	
	if(indexFile.Read(file_buffer.GetBufferInterface()))
	{
		GTSL::Extract(audioResourceInfos, file_buffer);
	}
	else
	{
		GTSL::File packageFile; packageFile.Open(package_path, GTSL::File::WRITE, false);

		GTSL::FileQuery file_query(query_path);

		while(file_query.DoQuery())
		{
			auto file_path = resources_path;
			file_path += file_query.GetFileNameWithExtension();
			auto name = file_query.GetFileNameWithExtension(); name.Drop(name.FindLast('.').Get().Second);
			const auto hashed_name = GTSL::Id64(name);

			if (!audioResourceInfos.Find(hashed_name))
			{
				GTSL::File query_file;
				query_file.Open(file_path, GTSL::File::READ, false);

				GTSL::Buffer<BE::TAR> wavBuffer; wavBuffer.Allocate(query_file.GetSize(), 8, GetTransientAllocator());

				query_file.Read(wavBuffer.GetBufferInterface());

				AudioDataSerialize data;

				uint8 riff[4];                      // RIFF string
				uint32 overall_size = 0;               // overall size of file in bytes
				uint8 wave[4];                      // WAVE string
				uint8 fmt_chunk_marker[4];          // fmt string with trailing null char
				uint32 length_of_fmt = 0;                 // length of the format data
				uint16 format_type = 0;                   // format type. 1-PCM, 3- IEEE float, 6 - 8bit A law, 7 - 8bit mu law
				uint16 channels = 0;                      // no.of channels
				uint32 sample_rate = 0;                   // sampling rate (blocks per second)
				uint32 byte_rate = 0;                      // SampleRate * NumChannels * BitsPerSample/8
				uint16 block_align = 0;                   // NumChannels * BitsPerSample/8
				uint16 bits_per_sample = 0;               // bits per sample, 8- 8bits, 16- 16 bits etc
				uint8 data_chunk_header[4];        // DATA string or FLLR string
				uint32 data_size = 0;                     // NumSamples * NumChannels * BitsPerSample/8 - size of the next chunk that will be read

				wavBuffer.ReadBytes(4, riff);
				BE_ASSERT(riff[0] == 'R' && riff[1] == 'I' && riff[2] == 'F' && riff[3] == 'F', "No RIFF");

				Extract(overall_size, wavBuffer);
				wavBuffer.ReadBytes(4, wave); BE_ASSERT(wave[0] == 'W' && wave[1] == 'A' && wave[2] == 'V' && wave[3] == 'E', "No WAVE");
				wavBuffer.ReadBytes(4, fmt_chunk_marker); BE_ASSERT(fmt_chunk_marker[0] == 'f' && fmt_chunk_marker[1] == 'm' && fmt_chunk_marker[2] == 't' && fmt_chunk_marker[3] == 32, "No fmt");
				Extract(length_of_fmt, wavBuffer); BE_ASSERT(length_of_fmt == 16, "Unsupported");
				Extract(format_type, wavBuffer); BE_ASSERT(format_type == 1, "Format is not PCM, unsupported!");
				Extract(channels, wavBuffer);
				Extract(sample_rate, wavBuffer);
				Extract(byte_rate, wavBuffer); //(Sample Rate * BitsPerSample * Channels) / 8.
				Extract(block_align, wavBuffer);
				Extract(bits_per_sample, wavBuffer);

				data.ChannelCount = static_cast<uint8>(channels);
				data.SampleRate = sample_rate;
				data.BitDepth = static_cast<uint8>(bits_per_sample);

				wavBuffer.ReadBytes(4, data_chunk_header); BE_ASSERT(data_chunk_header[0] == 'd' && data_chunk_header[1] == 'a' && data_chunk_header[2] == 't' && data_chunk_header[3] == 'a', "No data");
				Extract(data_size, wavBuffer);

				data.Frames = data_size / channels / (bits_per_sample / 8);

				data.ByteOffset = (uint32)packageFile.GetSize();

				packageFile.Write(GTSL::Range<const byte*>(data_size, wavBuffer.GetData() + wavBuffer.GetReadPosition()));

				audioResourceInfos.Emplace(hashed_name, data);
			}
		}

		file_buffer.Resize(0);
		GTSL::Insert(audioResourceInfos, file_buffer);
		indexFile.Write(file_buffer);
	}

	initializePackageFiles(packageFiles, package_path);
}

AudioResourceManager::~AudioResourceManager()
{
}
