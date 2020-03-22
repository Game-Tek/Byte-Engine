#include "AudioCore.h"

#include "Math/BEM.hpp"

float dBToVolume(const float db) { return BEM::Power(10.0f, 0.05f * db); }

float VolumeTodB(const float volume) { return 20.0f * BEM::Log10(volume); }
