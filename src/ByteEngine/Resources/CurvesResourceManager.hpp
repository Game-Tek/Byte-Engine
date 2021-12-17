#include "ByteEngine/Core.h"

#include <GTSL/Math/Math.hpp>
#include <GTSL/Vector.hpp>

class CurvesResourceManager {
	struct CurveHandle { uint32 in; };

	enum class PlayModes {
		STOP, WRAP_AROUND, BOUNCE
	};

	CurveHandle CreateCurveInstance(Id name, PlayModes playMode = PlayModes::STOP, float32 timeScale = 1.f) {
		auto index = instances.Emplace();
		auto& instance = instances[index];
		instance.PlayMode = playMode; instance.timeScale = timeScale;
		return CurveHandle(index);
	}

	float32 Evaluate(const CurveHandle curve_instance_handle, const float32 deltaTime) {
		auto& instance = instances[curve_instance_handle.in];
		//const CurveInstance::TimePoint& pointsInTime = { time[GTSL::Math::Clamp(pos - 1u, 0, time)], time[GTSL::Math::Clamp(pos - 1u, 0, time)], time[GTSL::Math::Clamp(pos - 1u, 0, time)], time[pos] };
		float32 currentTime = 0;

		switch (instance.PlayMode) {
		case PlayModes::STOP:
			currentTime = GTSL::Math::Clamp(instance.currentTime + deltaTime, 0.0f, 1.0f);
			break;
		case PlayModes::WRAP_AROUND:
			currentTime = GTSL::Math::Modulo(instance.currentTime + deltaTime, 1.0f);
			break;
		case PlayModes::BOUNCE:
			instance.currentTime = GTSL::Math::Modulo(instance.currentTime + deltaTime, 1.0f * 2.0f);
			currentTime -= GTSL::Math::Clamp(instance.currentTime - 1.0f, 0.0f, 1.0f);
			break;
		}

		uint32 pos = 0;

		{
			GTSL::StaticVector<uint32, 4> stack;
			while (pos < instance.TimePoints && instance.TimePoints[pos].X <= currentTime && stack.GetLength() < 4) { ++pos; stack.EmplaceBack(pos); }
			//todo: clear stack when changed curve segment
		}

		GTSL::StaticVector<float32, 4> points;
		const CurveInstance::TimePoint* pointsInTime[] = { &instance.TimePoints[pos - 3u], &instance.TimePoints[pos - 2u], &instance.TimePoints[pos - 1u], &instance.TimePoints[pos] };

		for(auto e : pointsInTime) { points.EmplaceBack(e->point.y); }

		//GTSL::Math::Lerp(points[0].y, point[1].y, (currentTime - times[0]) / (times[1] - times[0]));

		return EvaulateCubicBezier(points, (currentTime - pointsInTime[0]->X) / (pointsInTime[3]->X - pointsInTime[0]->X));
	}

	static float32 EvaulateCubicBezier(const GTSL::Range<const float32*> points, float32 t) {
		return GTSL::Math::Lerp(GTSL::Math::Lerp(GTSL::Math::Lerp(points[0], points[1], t), GTSL::Math::Lerp(points[1], points[2], t), t), GTSL::Math::Lerp(GTSL::Math::Lerp(points[1], points[2], t), GTSL::Math::Lerp(points[2], points[3], t), t), t);
	}

private:
	struct CurveInstance {
		struct TimePoint {
			struct Point {
				bool isCP = false;
				float32 y = 0.0f;
			} point;

			float32 X = 0.0f;
		};
		GTSL::StaticVector<TimePoint, 8> TimePoints;

		float32 currentTime = 0.0f, timeScale;
		PlayModes PlayMode;
	};

	GTSL::FixedVector<CurveInstance, BE::PAR> instances;
};