#include <GameStudio.h>

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{

	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}