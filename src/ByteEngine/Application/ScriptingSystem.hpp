#pragma once

/*
#include <mono/jit/jit.h>
#include <mono/metadata/assembly.h>
#include <mono/metadata/threads.h>
#include <mono/metadata/debug-helpers.h>

#include "ByteEngine/Object.h"
#include "ByteEngine/Game/System.hpp"
#include "GTSL/Vector.hpp"

struct ScriptingSystem : public BE::System
{
	ScriptingSystem(const InitializeInfo& initialize_info) : System(initialize_info, u8"ScriptingEngine") {
#if BE_PLATFORM_WINDOWS
		mono_set_dirs("C:/Program Files/Mono/lib", "C:/Program Files/Mono/etc");
#endif

		domain = mono_jit_init_version("ByteEngine", "v4.0.30319");

		if(!domain) {
			BE_LOG_ERROR(u8"Failed to initialize scripting VM.")
			return;
		}

		MonoAssembly* assembly = mono_domain_assembly_open(domain, "ByteEngine.dll");
		if (!assembly) {
			BE_LOG_ERROR(u8"Failed to open C# script module.")
			return;
		}

		image = mono_assembly_get_image(assembly);

		if(!image) {
			BE_LOG_ERROR(u8"Failed to get image from assembly.")
			return;
		}

		InvokeStaticMethod(u8"BE.EntryPoint:Print()");
	}

	template<void(*F)()>
	void BindNativeMethodToScriptMethod(const GTSL::StringView fully_qualified_name) {
		mono_add_internal_call(reinterpret_cast<const char*>(fully_qualified_name.GetData()), (void*)F);
	}

	void AttachThread() {
		auto* monoThread = mono_thread_attach(domain);
	}

	void InvokeStaticMethod(const GTSL::StringView fully_qualified_name) {
		MonoMethodDesc* monoMethodDesc = mono_method_desc_new(reinterpret_cast<const char*>(fully_qualified_name.GetData()), false);
		if (!monoMethodDesc) {
			BE_LOG_ERROR(u8"Failed to invoke script method: ", fully_qualified_name)
			return;
		}

		MonoMethod* method = mono_method_desc_search_in_image(monoMethodDesc, image);
		if (!method) {
			BE_LOG_ERROR(u8"Failed to invoke script method: ", fully_qualified_name)
			return;
		}

		//run the method
		MonoObject* returnMonoObject = mono_runtime_invoke(method, nullptr, nullptr, nullptr);
	}

	~ScriptingSystem() {
		mono_jit_cleanup(domain);
		//Note that for current versions of Mono, the mono runtime can�t be reloaded into the same process, so call mono_jit_cleanup() only if you�re never going to initialize it again.
	}

	void InvokeMethod() {
		const char8_t* className = u8"Dog";

		MonoClass* monoClass = mono_class_from_name(image, "BE", reinterpret_cast<const char*>(className));
		if (!monoClass) {
			BE_LOG_ERROR(u8"Failed to find script class: ", className)
			return;
		}

		MonoObject* monoObject = mono_object_new(domain, monoClass);
		if (!monoClass) {
			return;
		}

		// Call its default constructor
		mono_runtime_object_init(monoObject);

		//Build a method description object
		MonoObject* result;
		const char* BarkMethodDescStr = "Dog:Bark(int)";
		MonoMethodDesc* BarkMethodDesc = mono_method_desc_new(BarkMethodDescStr, false);
		if (!BarkMethodDesc) {
			return;
		}

		MonoMethod* method = mono_method_desc_search_in_image(BarkMethodDesc, image);
		if (!method) {
			return;
		}

		//Set the arguments for the method
		void* args[1];
		int barkTimes = 3;
		args[0] = &barkTimes;

		//Run the method
		mono_runtime_invoke(method, monoObject, args, nullptr);
	}

	void SetInstanceVariableValue() {
		const char8_t* fieldName = u8"";
		// find the Id field in the Entity class
		MonoClassField* idField = mono_class_get_field_from_name(monoClass, reinterpret_cast<const char*>(fieldName));

		int value = 42;

		// set the field's value
		mono_field_set_value(monoObject, idField, &value);

		int result;
		mono_field_get_value(monoObject, idField, &result);
	}

private:
	MonoDomain* domain = nullptr;
	MonoImage* image = nullptr;
	MonoObject* monoObject = nullptr;
	MonoClass* monoClass = nullptr;
};*/