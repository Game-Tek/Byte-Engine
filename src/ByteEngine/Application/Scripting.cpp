#include <mono/jit/jit.h>
#include <mono/metadata/threads.h>
#include <mono/metadata/assembly.h>

#include "ByteEngine/Object.h"
#include "ByteEngine/Game/System.h"
#include "GTSL/Vector.hpp"

struct ScriptingEngine : public System
{
	ScriptingEngine() : domain(mono_jit_init_version("myapp", "v4.0.30319")) {
		MonoAssembly* assembly = mono_domain_assembly_open(domain, "file.exe");
		if (!assembly) {
			BE_LOG_ERROR(u8"Failed to initialize C# script module.");
			return;
		}
	}

	template<void(*F)()>
	void RegisterCall() {
		mono_add_internal_call("Hello::Sample", (void*)F);
	}

	void AttachThread() {
		auto ss = mono_thread_attach(domain);
	}

	void Invoke() {
		MonoMethod* a; void* obj;
		GTSL::StaticVector<void*, 8> params;
		MonoObject** exec;
		mono_runtime_invoke(a, obj, params.begin(), exec);
	}

	~ScriptingEngine() {
		mono_jit_cleanup(domain);
		//Note that for current versions of Mono, the mono runtime can’t be reloaded into the same process, so call mono_jit_cleanup() only if you’re never going to initialize it again.
	}


	void t() {
		auto clss = mono_class_from_name({}, "BE", "ApplicationManager");
		auto method = mono_class_get_method_from_name(clss, "AddTask", 5);
		method = mono_object_get_virtual_method({}, method);
	}

private:
	MonoDomain* domain = nullptr;
};
