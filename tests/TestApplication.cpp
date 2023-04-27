#include <ByteEngine.h>

class TestApplication : public BE::Application {
public:
	TestApplication() : BE::Application(u8"Test Application") {
	}

	~TestApplication() {}

	bool initialize() override {
		BE::Application::initialize();
		return true;
	}

	void shutdown() override {
		BE::Application::shutdown();
	}

	virtual GTSL::ShortString<128> GetApplicationName() {
		return u8"Test Application";
	}
};

int CreateApplication(GTSL::Range<const GTSL::StringView*> arguments) {
//int CreateApplication() {
	auto application = TestApplication();

	int exitCode = -1;
	
	if (application.base_initialize(arguments)) //call BE::Application initialize, which does basic universal startup
	{
		if (application.initialize()) //call BE::Application virtual initialize which will call the chain of initialize's
		{
		}
	}

	application.shutdown();

	return exitCode; //Return and exit.
}