#pragma once

class Box
{
	float width = 0;
	float height = 0;
	float depth = 0;

public:
	[[nodiscard]] auto& GetWidth() { return width; }
	[[nodiscard]] auto& GetHeight() { return height; }
	[[nodiscard]] auto& GetDepth() { return depth; }

	[[nodiscard]] auto& GetWidth()  const { return width; }
	[[nodiscard]] auto& GetHeight() const { return height; }
	[[nodiscard]] auto& GetDepth()  const { return depth; }
	
	void SetWidthHeightDepth(const float _Width, const float _Height, const float _Depth) { width = _Width, height = _Height, depth = _Depth; }
};