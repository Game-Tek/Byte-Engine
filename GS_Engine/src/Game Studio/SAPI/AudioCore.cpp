#include "AudioCore.h"

#include "Math/GSM.hpp"

float dBToVolume(const float db) { return GSM::Power(10.0f, 0.05f * db); }

float VolumeTodB(const float volume) { return 20.0f * GSM::Log10(volume); }
