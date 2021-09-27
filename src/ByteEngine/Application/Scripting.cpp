//#include <mono/jit/jit.h>
//#include <mono/metadata/assembly.h>

struct MonoDomain;
struct MonoAssembly;
void mono_jit_init_version(char*, char*);
void mono_domain_assembly_open(MonoDomain*, char*);
void mono_jit_cleanup(MonoDomain*);
void mono_add_internal_call(char*, MonoAssembly*);

struct ScriptingEngine
{
	ScriptingEngine() {
		domain = mono_jit_init_version("myapp", "v4.0.30319");

		MonoAssembly* assembly;

		assembly = mono_domain_assembly_open(domain, "file.exe");
		if (!assembly)
	}

	~ScriptingEngine() {
		mono_jit_cleanup(domain);
	}

	MonoDomain* domain;
};