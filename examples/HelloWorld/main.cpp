#include <iostream>
#include <ByteEngine.h>

class HelloWorldApp final : public BE::Application
{
public:
    HelloWorldApp() : BE::Application(u8"Hello World") {}
    ~HelloWorldApp() {}

    bool initialize() override
    {
        BE::Application::initialize();
        return true;
    }

    void shutdown() override
    {
        BE::Application::shutdown();
    }

    GTSL::ShortString<128> GetApplicationName() override
    {
        return { u8"Hello World" };
    }
};

int CreateApplication()
{
    auto app = HelloWorldApp();

    auto exitCode = -1;
    if(app.base_initialize({}))
    {
        if(app.initialize())
        {

        }
    }

    app.shutdown();

    return exitCode;
}